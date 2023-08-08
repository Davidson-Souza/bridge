use std::str::FromStr;

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures::{channel::mpsc::Sender, lock::Mutex, SinkExt};
use rustreexo::accumulator::node_hash::NodeHash;
use crate::prover::{Requests, Responses};

struct AppState {
    sender: Mutex<
        Sender<(
            Requests,
            futures::channel::oneshot::Sender<Result<Responses, String>>,
        )>,
    >,
}

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
async fn get_block_by_height(height: web::Path<u32>, data: web::Data<AppState>) -> impl Responder {
    let height = height.into_inner();
    let res = perform_request(&data, Requests::GetBlockByHeight(height)).await;
    match res {
        Ok(Responses::Block(block)) => HttpResponse::Ok().body(hex::encode(block)),
        Ok(_) => HttpResponse::InternalServerError().body("Invalid response"),
        Err(e) => HttpResponse::NotAcceptable().body(e),
    }
}
async fn get_roots(data: web::Data<AppState>) -> HttpResponse {
    let res = perform_request(&data, Requests::GetRoots).await;
    match res {
        Ok(Responses::Roots(roots)) => HttpResponse::Ok().json(roots),
        Ok(_) => HttpResponse::InternalServerError().body("Invalid response"),
        Err(e) => HttpResponse::NotAcceptable().body(e),
    }
}
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
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
