#[macro_use]
extern crate tracing;

use clap::Parser;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rouille::Server;
use rouille::{Request, Response};
use str0m::change::SdpOffer;
use str0m::{Candidate, Input, Rtc, RtcError};

mod client;

use client::*;

#[derive(Default, Debug, Copy, Clone, clap::ValueEnum)]
enum Level {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl From<Level> for log::LevelFilter {
    fn from(level: Level) -> Self {
        match level {
            Level::Error => log::LevelFilter::Error,
            Level::Warn => log::LevelFilter::Warn,
            Level::Info => log::LevelFilter::Info,
            Level::Debug => log::LevelFilter::Debug,
            Level::Trace => log::LevelFilter::Trace,
        }
    }
}

#[derive(Parser)]
#[command(name = "SFU Server")]
#[command(author = "Rusty Rain <y@ngr.tc>")]
#[command(version = "0.1.0")]
#[command(about = "An example of SFU Server", long_about = None)]
struct Cli {
    #[arg(long, default_value_t = format!("127.0.0.1"))]
    host: String,
    #[arg(short, long, default_value_t = 8080)]
    signal_port: u16,
    #[arg(long, default_value_t = 3478)]
    media_port_min: u16,
    #[arg(long, default_value_t = 3495)]
    media_port_max: u16,

    #[arg(short, long)]
    debug: bool,
    #[arg(short, long, default_value_t = Level::Info)]
    #[clap(value_enum)]
    level: Level,
}

pub fn main() {
    let cli = Cli::parse();

    init_log();

    let certificate = include_bytes!("cer.pem").to_vec();
    let private_key = include_bytes!("key.pem").to_vec();

    // Figure out some public IP address, since Firefox will not accept 127.0.0.1 for WebRTC traffic.
    let media_ports: Vec<u16> = (cli.media_port_min..=cli.media_port_max).collect();
    let mut media_port_thread_map = HashMap::new();
    for port in media_ports {
        //let worker = wait_group.worker();
        let host = cli.host.clone();
        //let mut stop_rx = stop_rx.clone();
        let (signaling_tx, signaling_rx) = mpsc::sync_channel(1);

        // Spin up a UDP socket for the RTC. All WebRTC traffic is going to be multiplexed over this single
        // server socket. Clients are identified via their respective remote (UDP) socket address.
        let socket =
            UdpSocket::bind(format!("{host}:{port}")).expect(&format!("binding to {host}:{port}"));

        media_port_thread_map.insert(port, signaling_tx);

        // The run loop is on a separate thread to the web server.
        std::thread::spawn(move || run(socket, signaling_rx));
    }

    // The run loop is on a separate thread to the web server.
    //thread::spawn(move || run(socket, rx));
    let media_port_thread_map = Arc::new(media_port_thread_map);

    let host = cli.host.clone();
    let signal_port = cli.signal_port;
    let server = Server::new_ssl(
        format!("{host}:{signal_port}"),
        move |request| web_request(request, &host, media_port_thread_map.clone()),
        certificate,
        private_key,
    )
    .expect("starting the web server");

    let port = server.server_addr().port();
    info!("Connect a browser to https://{}:{}", cli.host, port);

    server.run();
}

fn init_log() {
    use std::env;
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "chat=info,str0m=info");
    }

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
}

// Handle a web request.
fn web_request(
    request: &Request,
    host: &str,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<Rtc>>>,
) -> Response {
    if request.method() == "GET" {
        return Response::html(include_str!("chat.html"));
    }

    let session_id = 0usize; //path[2].parse::<u64>().unwrap();
    let mut sorted_ports: Vec<u16> = media_port_thread_map.keys().map(|x| *x).collect();
    sorted_ports.sort();
    assert!(!sorted_ports.is_empty());
    let port = sorted_ports[(session_id as usize) % sorted_ports.len()];
    let tx = media_port_thread_map.get(&port).unwrap();

    // Expected POST SDP Offers.
    let mut data = request.data().expect("body to be available");

    let offer: SdpOffer = serde_json::from_reader(&mut data).expect("serialized offer");
    let mut rtc = Rtc::builder()
        // Uncomment this to see statistics
        // .set_stats_interval(Some(Duration::from_secs(1)))
        // .set_ice_lite(true)
        .build();

    // Add the shared UDP socket as a host candidate
    let candidate = Candidate::host(
        SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap(),
        "udp",
    )
    .expect("a host candidate");
    rtc.add_local_candidate(candidate);

    // Create an SDP Answer.
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .expect("offer to be accepted");

    // The Rtc instance is shipped off to the main run loop.
    tx.send(rtc).expect("to send Rtc instance");

    let body = serde_json::to_vec(&answer).expect("answer to serialize");

    Response::from_data("application/json", body)
}

/// This is the "main run loop" that handles all clients, reads and writes UdpSocket traffic,
/// and forwards media data between clients.
fn run(socket: UdpSocket, rx: Receiver<Rtc>) -> Result<(), RtcError> {
    let mut clients: Vec<Client> = vec![];
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0; 2000];

    loop {
        // Clean out disconnected clients
        clients.retain(|c| c.rtc.is_alive());

        // Spawn new incoming clients from the web server thread.
        if let Some(mut client) = spawn_new_client(&rx) {
            // Add incoming tracks present in other already connected clients.
            for track in clients.iter().flat_map(|c| c.tracks_in.iter()) {
                let weak = Arc::downgrade(&track.id);
                client.handle_track_open(weak);
            }

            clients.push(client);
        }

        // Poll clients until they return timeout
        let mut timeout = Instant::now() + Duration::from_millis(100);
        for client in clients.iter_mut() {
            let t = poll_until_timeout(client, &mut to_propagate, &socket);
            timeout = timeout.min(t);
        }

        // If we have an item to propagate, do that
        if let Some(p) = to_propagate.pop_front() {
            propagate(&p, &mut clients);
            continue;
        }

        // The read timeout is not allowed to be 0. In case it is 0, we set 1 millisecond.
        let duration = (timeout - Instant::now()).max(Duration::from_millis(1));

        socket
            .set_read_timeout(Some(duration))
            .expect("setting socket read timeout");

        if let Some(input) = read_socket_input(&socket, &mut buf) {
            // The rtc.accepts() call is how we demultiplex the incoming packet to know which
            // Rtc instance the traffic belongs to.
            if let Some(client) = clients.iter_mut().find(|c| c.accepts(&input)) {
                // We found the client that accepts the input.
                client.handle_input(input);
            } else {
                // This is quite common because we don't get the Rtc instance via the mpsc channel
                // quickly enough before the browser send the first STUN.
                debug!("No client accepts UDP input: {:?}", input);
            }
        }

        // Drive time forward in all clients.
        let now = Instant::now();
        for client in &mut clients {
            client.handle_input(Input::Timeout(now));
        }
    }
}
