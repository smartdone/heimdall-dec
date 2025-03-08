use actix_files::Files;
use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;

#[derive(Deserialize)]
struct FormData {
    rpc: String,
    address: String,
}

#[post("/submit")]
async fn submit_form(form: web::Json<FormData>) -> impl Responder {
    let response_message = format!(
        "Received RPC: {} and Address: {}",
        form.rpc, form.address
    );
    HttpResponse::Ok().json(response_message)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(submit_form)
            .service(Files::new("/", "./static").index_file("index.html"))
    })
        .bind("127.0.0.1:8080")?
        .run()
        .await
}