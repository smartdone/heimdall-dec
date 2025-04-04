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
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

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

    let mut code = provider
        .get_code_at(target_address.clone())
        .await
        .map_err(|e| format!("Get code failed: {}", e))?;
    if is_proxy {
        code = provider
            .get_code_at(impl_address.clone())
            .await
            .map_err(|e| format!("Get code failed: {}", e))?;
    }

    let encoded_code = encode(code);

    let args = DecompilerArgsBuilder::new()
        .target(encoded_code)
        .include_solidity(true)
        .skip_resolving(true)
        .build()
        .unwrap();

    // 使用mpsc通道传递decompile结果
    let (tx, rx) = mpsc::channel();

    // 在单独线程中创建新的运行时，设置128MB栈空间
    thread::Builder::new()
        .stack_size(128 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let res = rt.block_on(async {
                decompile(args)
                    .await
                    .map_err(|e| format!("Decompile failed: {}", e))
            });
            tx.send(res).expect("发送decompile结果失败");
        })
        .expect("线程启动失败");

    // 避免阻塞异步线程，用spawn_blocking包装接收操作
    let decompiled_result =
        tokio::task::spawn_blocking(move || rx.recv().expect("接收decompile结果失败")).await?;

    let decompiled = match decompiled_result {
        Ok(result) => result,
        Err(e) => return Err(format!("Decompile error: {}", e).into()),
    };

    let source = decompiled
        .source
        .ok_or("Decompile failed: source is None")?;

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
