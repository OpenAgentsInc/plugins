use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{delete, get},
    Extension, Form, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast::{channel, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt as _};

pub type PluginsStream = Sender<PluginUpdate>;

#[derive(Clone, Serialize, Debug)]
pub enum MutationKind {
    Create,
    Delete,
}

#[derive(Clone, Serialize, Debug)]
pub struct PluginUpdate {
    mutation_kind: MutationKind,
    id: i32,
}

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

#[derive(sqlx::FromRow, Serialize, Deserialize)]
struct Plugin {
    id: i32,
    description: String,
    wasm_url: String,
}

#[derive(sqlx::FromRow, Serialize, Deserialize)]
struct PluginNew {
    description: String,
    wasm_url: String,
}

#[shuttle_runtime::main]
async fn main(#[shuttle_shared_db::Postgres] db: PgPool) -> shuttle_axum::ShuttleAxum {
    sqlx::migrate!()
        .run(&db)
        .await
        .expect("Looks like something went wrong with migrations :(");

    let (plugin_tx, _plugin_rx) = channel::<PluginUpdate>(10);
    let state = AppState { db };

    let router = Router::new()
        .route("/", get(home))
        .route("/stream", get(stream))
        .route("/styles.css", get(styles))
        .route("/plugins", get(fetch_plugins).post(create_plugin))
        .route("/plugins/:id", delete(delete_plugin))
        .route("/plugins/stream", get(handle_plugin_stream))
        .with_state(state)
        .layer(Extension(plugin_tx));

    Ok(router.into())
}

async fn home() -> impl IntoResponse {
    HelloTemplate
}

async fn stream() -> impl IntoResponse {
    StreamTemplate
}

async fn fetch_plugins(State(state): State<AppState>) -> impl IntoResponse {
    let plugins = sqlx::query_as::<_, Plugin>("SELECT * FROM PLUGINS")
        .fetch_all(&state.db)
        .await
        .unwrap();

    PluginRecords { plugins }
}

pub async fn styles() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/css")
        .body(include_str!("../templates/styles.css").to_owned())
        .unwrap()
}

async fn create_plugin(
    State(state): State<AppState>,
    Extension(tx): Extension<PluginsStream>,
    Form(form): Form<PluginNew>,
) -> impl IntoResponse {
    let plugin = sqlx::query_as::<_, Plugin>(
      "INSERT INTO PLUGINS (description, wasm_url) VALUES ($1, $2) RETURNING id, description, wasm_url",
  )
  .bind(form.description)
  .bind(form.wasm_url)
  .fetch_one(&state.db)
  .await
  .unwrap();

    if tx
        .send(PluginUpdate {
            mutation_kind: MutationKind::Create,
            id: plugin.id,
        })
        .is_err()
    {
        eprintln!(
            "Record with ID {} was created but nobody's listening to the stream!",
            plugin.id
        );
    }

    PluginNewTemplate { plugin }
}

async fn delete_plugin(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Extension(tx): Extension<PluginsStream>,
) -> impl IntoResponse {
    sqlx::query("DELETE FROM PLUGINS WHERE ID = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .unwrap();

    if tx
        .send(PluginUpdate {
            mutation_kind: MutationKind::Delete,
            id,
        })
        .is_err()
    {
        eprintln!(
            "Record with ID {} was deleted but nobody's listening to the stream!",
            id
        );
    }

    StatusCode::OK
}

#[derive(Template)]
#[template(path = "index.html")]
struct HelloTemplate;

#[derive(Template)]
#[template(path = "stream.html")]
struct StreamTemplate;

#[derive(Template)]
#[template(path = "plugins.html")]
struct PluginRecords {
    plugins: Vec<Plugin>,
}

#[derive(Template)]
#[template(path = "plugin.html")]
struct PluginNewTemplate {
    plugin: Plugin,
}

pub async fn handle_plugin_stream(
    Extension(tx): Extension<PluginsStream>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = tx.subscribe();

    let stream = BroadcastStream::new(rx);

    Sse::new(
        stream
            .map(|msg| {
                let msg = msg.unwrap();
                let json = format!("<div>{}</div>", json!(msg));
                Event::default().data(json)
            })
            .map(Ok),
    )
    .keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(600))
            .text("keep-alive-text"),
    )
}
