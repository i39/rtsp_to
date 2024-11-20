use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use gstreamer as gst;
use gstreamer::prelude::*;
use serde::Deserialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tera::{Context, Tera};

#[derive(Deserialize)]
struct RtspUrl {
    url: String,
}

struct AppState {
    pipelines: Mutex<Vec<gst::Pipeline>>,
    client_count: AtomicUsize,
}

impl AppState {
    fn new() -> Self {
        Self {
            pipelines: Mutex::new(Vec::new()),
            client_count: AtomicUsize::new(0),
        }
    }
}

async fn add_rtsp_stream(
    state: web::Data<Arc<AppState>>,
    rtsp: web::Json<RtspUrl>,
) -> impl Responder {
    gst::init().expect("Failed to initialize GStreamer");

    let pipeline_str = format!(
        "rtspsrc location={} ! decodebin ! x264enc tune=zerolatency ! mp4mux fragment-duration=4 ! \
         splitmuxsink name=sink dash-path=dash_output/manifest.mpd hls-playlist-location=hls_output/playlist.m3u8",
        rtsp.url
    );

    match gst::parse_launch(&pipeline_str) {
        Ok(pipeline) => {
            let pipeline = pipeline
                .downcast::<gst::Pipeline>()
                .expect("Expected a valid pipeline");

            pipeline
                .set_state(gst::State::Playing)
                .expect("Unable to set the pipeline to Playing");

            state.pipelines.lock().unwrap().push(pipeline);
            HttpResponse::Ok().body("RTSP stream added successfully")
        }
        Err(_) => HttpResponse::BadRequest().body("Failed to create RTSP pipeline"),
    }
}

async fn remove_rtsp_stream(
    state: web::Data<Arc<AppState>>,
    idx: web::Path<usize>,
) -> impl Responder {
    let mut pipelines = state.pipelines.lock().unwrap();
    if *idx < pipelines.len() {
        let pipeline = pipelines.remove(*idx);
        pipeline
            .set_state(gst::State::Null)
            .expect("Unable to set the pipeline to Null");
        HttpResponse::Ok().body("RTSP stream removed successfully")
    } else {
        HttpResponse::BadRequest().body("Invalid stream index")
    }
}

async fn get_streams(state: web::Data<Arc<AppState>>, tmpl: web::Data<Tera>) -> impl Responder {
    let pipelines = state.pipelines.lock().unwrap();
    let streams: Vec<_> = pipelines.iter().enumerate().map(|(i, _)| i).collect();

    let mut ctx = Context::new();
    ctx.insert("streams", &streams);

    match tmpl.render("streams.html", &ctx) {
        Ok(body) => HttpResponse::Ok().content_type("text/html").body(body),
        Err(_) => HttpResponse::InternalServerError().body("Template error"),
    }
}

async fn get_stream_status(
    state: web::Data<Arc<AppState>>,
    idx: web::Path<usize>,
) -> impl Responder {
    let pipelines = state.pipelines.lock().unwrap();
    if *idx < pipelines.len() {
        let pipeline = &pipelines[*idx];
        let state = pipeline.current_state();
        let status = match state {
            gst::State::Playing => "Playing",
            gst::State::Paused => "Paused",
            gst::State::Ready => "Ready",
            gst::State::Null => "Null",
            _ => "Unknown",
        };
        HttpResponse::Ok().body(format!("Stream {} is currently in state: {}", idx, status))
    } else {
        HttpResponse::BadRequest().body("Invalid stream index")
    }
}

async fn get_client_count(state: web::Data<Arc<AppState>>) -> impl Responder {
    let count = state.client_count.load(Ordering::Relaxed);
    HttpResponse::Ok().body(format!("Current client count: {}", count))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let state = Arc::new(AppState::new());

    let tera = Tera::new("templates/**/*").expect("Failed to initialize Tera templates");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .app_data(web::Data::new(tera.clone()))
            .route("/", web::get().to(get_streams))
            .route("/add", web::post().to(add_rtsp_stream))
            .route("/remove/{idx}", web::delete().to(remove_rtsp_stream))
            .route("/status/{idx}", web::get().to(get_stream_status))
            .route("/clients", web::get().to(get_client_count))
    })
    .bind("0.0.0.0:8081")?
    .run()
    .await
}
