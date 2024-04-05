//SPDX-License-Identifier: MIT

//! This is a simple REST API that can be used to query Utreexo data. You can get the roots
//! of the accumulator, get a proof for a leaf, and get a block and the associated UData.
use std::str::FromStr;
use std::sync::Arc;

use actix_cors::Cors;
use actix_web::web;
use actix_web::App;
use actix_web::HttpResponse;
use actix_web::HttpServer;
use actix_web::Responder;
use bitcoin::consensus::deserialize;
use bitcoin::network::utreexo::UtreexoBlock;
use bitcoin::Block;
use bitcoin::BlockHash;
use bitcoin::Txid;
use bitcoin_hashes::Hash;
use bitcoincore_rpc::jsonrpc::serde_json::json;
use futures::channel::mpsc::Sender;
use futures::lock::Mutex;
use futures::SinkExt;
use rustreexo::accumulator::node_hash::NodeHash;
use rustreexo::accumulator::proof::Proof;
use serde::Deserialize;
use serde::Serialize;

use crate::chainview::ChainView;
use crate::prover::Requests;
use crate::prover::Responses;
/// This is the state of the actix-web server that will be passed as reference by each
/// callback function. It contains a sender that can be used to send requests to the prover.
struct AppState {
    /// Sender to send requests to the prover.
    sender: Mutex<
        Sender<(
            Requests,
            futures::channel::oneshot::Sender<Result<Responses, String>>,
        )>,
    >,
    view: Arc<ChainView>,
}

/// This function is used to send a request to the prover and wait for the response, and
/// return the response or an error.
async fn perform_request(
    data: &web::Data<AppState>,
    request: Requests,
) -> Result<Responses, String> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    data.sender
        .lock()
        .await
        .send((request, sender))
        .await
        .unwrap();
    receiver.await.unwrap()
}

/// The handler for the `/proof/{hash}` endpoint. It returns a proof for the given hash, if
/// it exists.
async fn get_proof(hash: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let hash = hash.into_inner();
    let hash = NodeHash::from_str(&hash);
    if let Err(e) = hash {
        return HttpResponse::BadRequest().body(format!("Invalid hash {e}"));
    }
    let res = perform_request(&data, Requests::GetProof(hash.unwrap())).await;

    match res {
        Ok(Responses::Proof(proof)) => HttpResponse::Ok().json(json!({
            "error": null,
            "data": JsonProof::from(proof),
        })),
        Ok(_) => HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response",
            "data": null
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": e,
            "data": null
        })),
    }
}
async fn get_transaction(hash: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let hash = hash.into_inner();
    let hash = Txid::from_str(&hash);
    if let Err(e) = hash {
        return HttpResponse::BadRequest().body(format!("Invalid hash {e}"));
    }
    let res = perform_request(&data, Requests::GetTransaction(hash.unwrap())).await;

    match res {
        Ok(Responses::Transaction((tx, proof))) => HttpResponse::Ok().json(json!({
            "error": null,
            "data": {
                "tx": tx,
                "proof": JsonProof::from(proof),
            },
        })),
        Ok(_) => HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response",
            "data": null
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": e,
            "data": null
        })),
    }
}
/// The handler for the `/block/{height}` endpoint. It returns the block at the given height.
async fn get_block_by_height(height: web::Path<u32>, data: web::Data<AppState>) -> impl Responder {
    let height = height.into_inner();
    let res = perform_request(&data, Requests::GetBlockByHeight(height)).await;
    match res {
        Ok(Responses::Block(block)) => {
            let block: UBlock = deserialize::<UtreexoBlock>(&block).unwrap().into();
            HttpResponse::Ok().json(json!({ "error": null, "data": block}))
        }
        Ok(_) => HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response from backend",
            "data": null
        })),
        Err(e) => HttpResponse::NotAcceptable().json(json!({
            "error": e,
            "data": null
        })),
    }
}
// Returns n blocks starting from the given height
async fn get_n_blocks(height: web::Path<(u32, u32)>, data: web::Data<AppState>) -> impl Responder {
    let (height, n) = height.into_inner();
    let res = perform_request(&data, Requests::GetBlocksByHeight(height, n)).await;
    match res {
        Ok(Responses::Blocks(blocks)) => {
            let blocks: Vec<UBlock> = blocks
                .into_iter()
                .map(|block| deserialize::<UtreexoBlock>(&block).unwrap().into())
                .collect();
            HttpResponse::Ok().json(json!({ "error": null, "data": blocks}))
        }
        Ok(_) => HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response from backend",
            "data": null
        })),
        Err(e) => HttpResponse::NotAcceptable().json(json!({
            "error": e,
            "data": null
        })),
    }
}
/// Same as `get_roots`, but returns the leaf number of the accumulator too.
async fn get_roots_with_leaf(data: web::Data<AppState>) -> Result<HttpResponse, actix_web::Error> {
    let res = perform_request(&data, Requests::GetCSN).await;
    match res {
        Ok(Responses::CSN(acc)) => Ok(HttpResponse::Ok().json(json!({
            "error": null,
            "data": acc
        }))),
        Ok(_) => Ok(HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response",
            "data": null
        }))),
        Err(e) => Ok(HttpResponse::NotAcceptable().json(json!({
            "error": e,
            "data": null
        }))),
    }
}
/// The handler for the `/roots` endpoint. It returns the roots of the accumulator.
async fn get_roots(data: web::Data<AppState>) -> HttpResponse {
    let res = perform_request(&data, Requests::GetRoots).await;
    match res {
        Ok(Responses::Roots(roots)) => {
            let roots = roots.iter().map(|x| x.to_string()).collect::<Vec<String>>();

            HttpResponse::Ok().json(json!({
                "error": null,
                "data": roots
            }))
        }
        Ok(_) => HttpResponse::InternalServerError().json(json!({
            "error": "Invalid response",
            "data": null
        })),
        Err(e) => HttpResponse::NotAcceptable().json(json!({
            "error": e,
            "data": null
        })),
    }
}

async fn get_roots_for_height(
    hash: web::Path<BlockHash>,
    data: web::Data<AppState>,
) -> HttpResponse {
    let hash = hash.into_inner();
    match data.view.get_acc(hash) {
        Ok(Some(acc)) => {
            let acc = acc.iter().map(|x| x.to_string()).collect::<Vec<String>>();
            HttpResponse::Ok().json(json!({
                "error": null,
                "data": acc
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(json!({
            "error": "No roots found for this block",
            "data": null
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "error": e.to_string(),
            "data": null
        })),
    }
}

/// This function creates the actix-web server and returns a future that can be awaited.
pub async fn create_api(
    request: Sender<(
        Requests,
        futures::channel::oneshot::Sender<Result<Responses, String>>,
    )>,
    view: Arc<ChainView>,
) -> std::io::Result<()> {
    let app_state = web::Data::new(AppState {
        sender: Mutex::new(request),
        view,
    });
    HttpServer::new(move || {
        let cors = Cors::permissive();
        App::new()
            .wrap(cors)
            .app_data(app_state.clone())
            .route("/prove/{leaf}", web::get().to(get_proof))
            .route("/roots", web::get().to(get_roots))
            .route("/block/{height}", web::get().to(get_block_by_height))
            .route("/tx/{hash}/outputs", web::get().to(get_transaction))
            .route("/acc", web::get().to(get_roots_with_leaf))
            .route("/batch_block/{height}/{n}", web::get().to(get_n_blocks))
            .route("/roots/{hash}", web::get().to(get_roots_for_height))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
/// The proof serialization by serde-json is not very nice, because it serializes byte-arrays
/// as a array of integers. This struct is used to serialize the proof in a nicer way.
#[derive(Clone, Serialize, Deserialize)]
struct JsonProof {
    targets: Vec<u64>,
    hashes: Vec<String>,
}

impl From<Proof> for JsonProof {
    fn from(proof: Proof) -> Self {
        let targets = proof.targets;
        let mut hashes = Vec::new();
        for hash in proof.hashes {
            hashes.push(hash.to_string());
        }
        JsonProof { targets, hashes }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct UBlock {
    block: Block,
    proof: JsonProof,
    leaf_data: Vec<LeafData>,
}
#[derive(Clone, Serialize, Deserialize)]
struct LeafData {
    /// Header code tells the height of creating for this UTXO and whether it's a coinbase
    pub header_code: u32,
    /// The amount locked in this UTXO
    pub amount: u64,
    /// The type of the locking script for this UTXO
    pub spk_ty: ScriptPubkeyType,
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub enum ScriptPubkeyType {
    /// An non-specified type, in this case the script is just copied over
    Other(Box<[u8]>),
    /// p2pkh
    PubKeyHash,
    /// p2wsh
    WitnessV0PubKeyHash,
    /// p2sh
    ScriptHash,
    /// p2wsh
    WitnessV0ScriptHash,
}
impl From<UtreexoBlock> for UBlock {
    fn from(block: UtreexoBlock) -> Self {
        let proof = block.udata.as_ref().unwrap().proof.clone();
        let proof = Proof {
            hashes: proof
                .hashes
                .iter()
                .map(|x| NodeHash::from(x.as_inner()))
                .collect(),
            targets: proof.targets.iter().map(|x| x.0).collect(),
        }
        .into();

        let leaves = block.udata.clone().unwrap().leaves.clone();
        let block = block.block;

        let leaf_data = unsafe { std::mem::transmute(leaves) };
        Self {
            block,
            proof,
            leaf_data,
        }
    }
}
