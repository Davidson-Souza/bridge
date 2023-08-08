use std::str::FromStr;

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures::{channel::mpsc::Sender, lock::Mutex, SinkExt};
use rustreexo::accumulator::node_hash::NodeHash;
use serde::{Deserialize, Serialize};

use crate::prover::{Requests, Responses};

// Simple User struct for demonstration purposes
#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: i32,
    name: String,
    email: String,
}

// In-memory database simulation
struct AppState {
    sender: Mutex<Sender<(Requests, futures::channel::oneshot::Sender<Responses>)>>,
}
async fn perform_request(data: &web::Data<AppState>, request: Requests) -> Option<Responses> {
    let (sender, receiver) = futures::channel::oneshot::channel();
    data.sender
        .lock()
        .await
        .send((request, sender))
        .await
        .unwrap();
    receiver.await.ok()
}
// Handler to get a user by ID
async fn get_proof(hash: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let hash = hash.into_inner();
    let hash = NodeHash::from_str(&hash);
    if let Err(e) = hash {
        return HttpResponse::BadRequest().body(format!("Invalid hash {e}"));
    }
    let res = perform_request(&data, Requests::GetProof(hash.unwrap())).await;

    if let Some(proof) = res {
        HttpResponse::Ok().json(proof)
    } else {
        HttpResponse::NotFound().body("User not found")
    }
}

pub async fn create_api(
    request: Sender<(Requests, futures::channel::oneshot::Sender<Responses>)>,
) -> std::io::Result<()> {
    // Simulate an in-memory database for users
    let app_state = web::Data::new(AppState {
        sender: Mutex::new(request),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/prove/{leaf}", web::get().to(get_proof))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
