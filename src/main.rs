// Copyright Rivtower Technologies LLC.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![forbid(unsafe_code)]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    unused_crate_dependencies,
    clippy::missing_const_for_fn,
    unused_extern_crates
)]

#[macro_use]
extern crate tracing;

mod config;

use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    extract::State, http::StatusCode, middleware, response::IntoResponse, routing::any, Json,
    Router,
};
use clap::Parser;
use ethers::{prelude::*, signers::coins_bip39::English, utils::keccak256};
use k256::ecdsa::SigningKey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

use config::Config;

use common_rs::{
    consul::{register_to_consul, ConsulClient},
    restful::{handle_http_error, ok, RESTfulError},
    sm::{sm2_public_key, sm2_sign, sm2_verify},
};

fn clap_about() -> String {
    let name = env!("CARGO_PKG_NAME").to_string();
    let version = env!("CARGO_PKG_VERSION");
    let authors = env!("CARGO_PKG_AUTHORS");
    name + " " + version + "\n" + authors
}

#[derive(Parser)]
#[clap(version, about = clap_about())]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser)]
enum SubCommand {
    /// run this service
    #[clap(name = "run")]
    Run(RunOpts),
}

/// A subcommand for run
#[derive(Parser)]
struct RunOpts {
    /// Chain config path
    #[clap(short = 'c', long = "config", default_value = "config.toml")]
    config_path: String,
}

fn main() {
    ::std::env::set_var("RUST_BACKTRACE", "full");

    let opts: Opts = Opts::parse();

    match opts.subcmd {
        SubCommand::Run(opts) => {
            if let Err(e) = run(opts) {
                warn!("err: {:?}", e);
            }
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(handle_keys, handle_keys_addr, handle_sign, handle_verify,),
    components(schemas(RequestParams, CryptoType))
)]
struct ApiDoc;

#[derive(Clone)]
struct AppState {
    config: Config,
    _consul: Arc<RwLock<ConsulClient>>,
}

#[tokio::main]
async fn run(opts: RunOpts) -> Result<()> {
    ::std::env::set_var("RUST_BACKTRACE", "full");

    let config = Config::new(&opts.config_path);

    // init tracer
    cloud_util::tracer::init_tracer("kms".to_string(), &config.log_config)
        .map_err(|e| println!("tracer init err: {e}"))
        .unwrap();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));

    let consul = register_to_consul(
        config.consul_addr.clone(),
        config.service_name.clone(),
        config.port,
    )
    .await?;

    let app_state = AppState {
        config,
        _consul: Arc::new(RwLock::new(consul)),
    };

    let app = Router::new()
        .route("/api/keys", any(handle_keys))
        .route("/api/keys/addr", any(handle_keys_addr))
        .route("/api/keys/sign", any(handle_sign))
        .route("/api/keys/verify", any(handle_verify))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route_layer(middleware::from_fn(handle_http_error))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "code": 404,
                    "message": "Not Found",
                })),
            )
        })
        .with_state(app_state);

    info!("kms listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .map_err(|e| anyhow::anyhow!("axum serve failed: {e}"))?;
    anyhow::bail!("unreachable!")
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
enum CryptoType {
    SM2,
    Secp256k1,
}

#[derive(Debug, Serialize, Default, Deserialize, ToSchema)]
#[serde(default)]
struct RequestParams {
    #[serde(skip_serializing_if = "String::is_empty")]
    user_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    crypto_type: Option<CryptoType>,
    #[serde(skip_serializing_if = "String::is_empty")]
    message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    address: String,
}

fn derive_wallet(master_key: &str, user_code: &str) -> Result<Wallet<SigningKey>, RESTfulError> {
    let user_code_hash = keccak256(user_code);
    let account = u32::from_be_bytes(user_code_hash[0..4].try_into().unwrap());
    let index = u32::from_be_bytes(user_code_hash[4..8].try_into().unwrap());
    let path = format!("m/44'/60'/{}'/0/{}", account, index);
    debug!("path: {}", path);
    let wallet = MnemonicBuilder::<English>::default()
        .phrase(master_key)
        .derivation_path(&path)?
        .build()
        .map_err(|e| anyhow::anyhow!("derive wallet failed: {e}"))?;
    Ok(wallet)
}

#[utoipa::path(
    post,
    path = "/api/keys",
    params(
        ("version" = String, Path, description = "api version")
    ),
    request_body = RequestParams,
)]
async fn handle_keys(
    State(state): State<AppState>,
    Json(params): Json<RequestParams>,
) -> Result<impl IntoResponse, RESTfulError> {
    debug!("params: {:?}", params);
    if params.user_code.is_empty() {
        return Err(anyhow::anyhow!("user_code missing").into());
    }

    let wallet = derive_wallet(&state.config.master_key, &params.user_code)?;

    let public_key = match params.crypto_type {
        Some(CryptoType::SM2) => {
            let privkey = wallet.signer().to_bytes();
            let public_key = sm2_public_key(&privkey)?;
            hex::encode_upper(public_key)
        }
        Some(CryptoType::Secp256k1) => {
            hex::encode_upper(wallet.signer().verifying_key().to_sec1_bytes())
        }
        None => return Err(anyhow::anyhow!("crypto_type missing").into()),
    };
    ok(json!({
        "user_code": params.user_code,
        "crypto_type": params.crypto_type,
        "address": wallet.address(),
        "public_key": public_key,
    }))
}

#[utoipa::path(
    post,
    path = "/api/{version}/keys/addr",
    params(
        ("version" = String, Path, description = "api version")
    ),
    request_body = RequestParams,
)]
async fn handle_keys_addr(
    Json(params): Json<RequestParams>,
) -> Result<impl IntoResponse, RESTfulError> {
    debug!("params: {:?}", params);
    if params.address.is_empty() {
        return Err(anyhow::anyhow!("address missing").into());
    }
    let wallet: Wallet<SigningKey> = params
        .address
        .parse()
        .map_err(|e| anyhow::anyhow!("address parse failed: {e}"))?;

    let public_key = match params.crypto_type {
        Some(CryptoType::SM2) => {
            let privkey = wallet.signer().to_bytes();
            let public_key = sm2_public_key(&privkey)?;
            hex::encode_upper(public_key)
        }
        Some(CryptoType::Secp256k1) => {
            hex::encode_upper(wallet.signer().verifying_key().to_sec1_bytes())
        }
        None => return Err(anyhow::anyhow!("crypto_type missing").into()),
    };
    ok(json!({
        "user_code": params.user_code,
        "crypto_type": params.crypto_type,
        "address": wallet.address(),
        "public_key": public_key,
    }))
}

#[utoipa::path(
    post,
    path = "/api/{version}/keys/sign",
    params(
        ("version" = String, Path, description = "api version")
    ),
    request_body = RequestParams,
)]
async fn handle_sign(
    State(state): State<AppState>,
    Json(params): Json<RequestParams>,
) -> Result<impl IntoResponse, RESTfulError> {
    debug!("params: {:?}", params);
    if params.user_code.is_empty() {
        return Err(anyhow::anyhow!("user_code missing").into());
    }
    if params.message.is_empty() {
        return Err(anyhow::anyhow!("message missing").into());
    }
    let wallet = derive_wallet(&state.config.master_key, &params.user_code)?;
    match params.crypto_type {
        Some(CryptoType::SM2) => {
            let privkey = wallet.signer().to_bytes();
            let signature = hex::encode(sm2_sign(
                &sm2_public_key(&privkey)?,
                &privkey,
                params.message.as_bytes(),
            )?);
            ok(json!({
                "signature": signature,
            }))
        }
        Some(CryptoType::Secp256k1) => {
            let signature = wallet
                .sign_message(params.message.as_bytes())
                .await
                .map_err(|e| anyhow::anyhow!("Secp256k1 sign message failed: {e}"))?;
            let signature = hex::encode(signature.to_vec());
            ok(json!({
                "signature": signature,
            }))
        }
        None => Err(anyhow::anyhow!("crypto_type missing").into()),
    }
}

#[utoipa::path(
    post,
    path = "/api/{version}/keys/verify",
    params(
        ("version" = String, Path, description = "api version")
    ),
    request_body = RequestParams,
)]
async fn handle_verify(
    State(state): State<AppState>,
    Json(params): Json<RequestParams>,
) -> Result<impl IntoResponse, RESTfulError> {
    debug!("params: {:?}", params);
    if params.user_code.is_empty() {
        return Err(anyhow::anyhow!("user_code missing").into());
    }
    if params.message.is_empty() {
        return Err(anyhow::anyhow!("message missing").into());
    }
    if params.signature.is_empty() {
        return Err(anyhow::anyhow!("signature missing").into());
    }
    match params.crypto_type {
        Some(CryptoType::SM2) => {
            let signature = hex::decode(params.signature)
                .map_err(|e| anyhow::anyhow!("signature decode failed: {e}"))?;
            let verify_result = sm2_verify(&signature, params.message.as_bytes())?;
            ok(verify_result)
        }
        Some(CryptoType::Secp256k1) => {
            let wallet = derive_wallet(&state.config.master_key, &params.user_code)?;
            let signature = hex::decode(params.signature)
                .map_err(|e| anyhow::anyhow!("signature decode failed: {e}"))?;
            let signature = Signature::try_from(signature.as_slice())
                .map_err(|e| anyhow::anyhow!("signature decode failed: {e:?}"))?;
            let verify_result = signature.verify(params.message, wallet.address()).is_ok();
            ok(verify_result)
        }
        None => Err(anyhow::anyhow!("crypto_type missing").into()),
    }
}
