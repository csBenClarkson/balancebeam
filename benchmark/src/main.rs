use actix_web::{get, web, App, HttpServer, Responder};
use std::env;
use std::process::exit;

#[get("/")]
async fn index() -> impl Responder {
    "Hello, World!"
}

#[get("/{name}")]
async fn hello(name: web::Path<String>) -> impl Responder {
    format!("Hello {}!", &name)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("Usage: {} <port>", args[0]);
        exit(1);
    }
    let port = match args[1].parse::<u16>() {
        Ok(p) => {p}
        Err(_) => { println!("Invaild port."); exit(1); }
    };
    HttpServer::new(|| App::new().service(index).service(hello))
        .bind(("127.0.0.1", port))?
        .run()
        .await
}