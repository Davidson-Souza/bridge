//SPDX-License-Identifier: MIT

//! This is a simple REST API that can be used to query Utreexo data. You can get the roots
//! of the accumulator, get a proof for a leaf, and get a block and the associated UData.
use std::str::FromStr;

use crate::prover::{Requests, Responses};
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures::{channel::mpsc::Sender, lock::Mutex, SinkExt};
use rustreexo::accumulator::node_hash::NodeHash;

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
        Ok(Responses::Proof(proof)) => HttpResponse::Ok().json(proof),
        Ok(_) => HttpResponse::InternalServerError().body("Invalid response"),
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}
/// The handler for the `/block/{height}` endpoint. It returns the block at the given height.
async fn get_block_by_height(height: web::Path<u32>, data: web::Data<AppState>) -> impl Responder {
    let height = height.into_inner();
    let res = perform_request(&data, Requests::GetBlockByHeight(height)).await;
    match res {
        Ok(Responses::Block(block)) => HttpResponse::Ok().body(hex::encode(block)),
        Ok(_) => HttpResponse::InternalServerError().body("Invalid response"),
        Err(e) => HttpResponse::NotAcceptable().body(e),
    }
}

/// The handler for the `/roots` endpoint. It returns the roots of the accumulator.
async fn get_roots(data: web::Data<AppState>) -> HttpResponse {
    let res = perform_request(&data, Requests::GetRoots).await;
    match res {
        Ok(Responses::Roots(roots)) => HttpResponse::Ok().json(roots),
        Ok(_) => HttpResponse::InternalServerError().body("Invalid response"),
        Err(e) => HttpResponse::NotAcceptable().body(e),
    }
}

/// This function creates the actix-web server and returns a future that can be awaited.
pub async fn create_api(
    request: Sender<(
        Requests,
        futures::channel::oneshot::Sender<Result<Responses, String>>,
    )>,
) -> std::io::Result<()> {
    let app_state = web::Data::new(AppState {
        sender: Mutex::new(request),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/prove/{leaf}", web::get().to(get_proof))
            .route("/roots", web::get().to(get_roots))
            .route("/block/{height}", web::get().to(get_block_by_height))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}