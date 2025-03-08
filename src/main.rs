use actix_files::Files;
use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use alloy::hex::encode;
use alloy::hex::FromHex;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::transports::http::reqwest::Url;
use heimdall_decompiler::{decompile, DecompilerArgsBuilder};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::Arc;

static SALT: Lazy<U256> = Lazy::new(|| {
    B256::from_hex("0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc")
        .map(|b| b.into())
        .unwrap_or_else(|_| U256::ZERO)
});

#[derive(Deserialize)]
struct FormData {
    rpc: String,
    address: String,
}

#[post("/submit")]
async fn submit_form(form: web::Json<FormData>) -> impl Responder {
    match process_form(form.into_inner()).await {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(e) => HttpResponse::InternalServerError().body(format!("Internal Server Error: {}", e)),
    }
}

async fn process_form(form: FormData) -> Result<String, Box<dyn std::error::Error>> {
    let rpc = form.rpc;
    let addr = form.address;
    let provider = create_provider(&rpc)?;

    let target_address = Address::from_hex(&addr)?;

    println!("RPC: {}, Address: {:?}", rpc, target_address);

    let (is_proxy, impl_address) = check_proxy(&provider, &target_address).await?;

    println!("Proxy address {:?}", impl_address);

    let code = provider
        .get_code_at(target_address.clone())
        .await
        .map_err(|e| format!("Get code failed: {}", e))?;

    let encoded_code = encode(code);

    let args = DecompilerArgsBuilder::new()
        .target(encoded_code)
        .include_solidity(true)
        .skip_resolving(true)
        .build()
        .unwrap();

    let decompiled = decompile(args)
        .await
        .map_err(|e| format!("Decompile failed: {}", e))?;

    println!("Decompiled {:?}", decompiled);

    let source = decompiled
        .source
        .ok_or("Decompile failed: source is None")?;

    println!("Source {:?}", source);

    let result = if is_proxy {
        format!(
            "// address: {}\n// proxy address: {}\n{}",
            target_address, impl_address, source
        )
    } else {
        format!("// address: {}\n{}", target_address, source)
    };

    Ok(result)
}

fn create_provider(rpc: &str) -> Result<Arc<dyn Provider + Send + Sync>, String> {
    let url = Url::parse(rpc).map_err(|e| format!("Invalid RPC URL: {}", e))?;
    Ok(Arc::new(ProviderBuilder::new().on_http(url)))
}

async fn check_proxy(
    provider: &Arc<dyn Provider + Send + Sync>,
    target_address: &Address,
) -> Result<(bool, Address), String> {
    let mut is_proxy = false;
    let mut impl_address = target_address.clone();

    if let Ok(s) = provider.get_storage_at(target_address.clone(), *SALT).await {
        let mut bytes: [u8; 20] = [0u8; 20];
        let u_bytes: [u8; 32] = s.to_be_bytes();
        bytes.copy_from_slice(&u_bytes[12..]);
        let ad = Address::from(bytes);
        if ad != Address::ZERO {
            impl_address = ad.clone();
            is_proxy = true;
        }
    }

    Ok((is_proxy, impl_address))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(submit_form)
            .service(Files::new("/", "./static").index_file("index.html"))
    })
    .workers(4)
    .worker_max_blocking_threads(128)
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
