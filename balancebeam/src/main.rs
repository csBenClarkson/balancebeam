mod request;
mod response;

use std::collections::{BTreeSet, HashMap};
use std::io::ErrorKind;
use std::iter::FromIterator;
use clap::Parser;
use rand::SeedableRng;
use rand::seq::IteratorRandom;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::time;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use http::StatusCode;


/// Get Unix timestamp (since 1970-01-01), panic if earlier that that.
macro_rules! timestamp {
    () => {
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
                    .expect("SystemTime before UNIX EPOCH!")
                    .as_secs()
    };
}

/// Contains information parsed from the command-line invocation of balancebeam. The Parser macros
/// provide a fancy way to automatically construct a command-line argument parser.
#[derive(Parser, Debug)]
#[command(about = "Fun with load balancing")]
struct CmdOptions {
    #[arg(short, long, default_value = "0.0.0.0:1100")]
    bind: String,
    #[arg(short, long)]
    upstream: Vec<String>,
    #[arg(long, default_value = "10")]
    active_health_check_interval: usize,
    #[arg(long, default_value = "/")]
    active_health_check_path: String,
    #[arg(long, default_value = "0", help = "Maximum number of requests to accept per IP per minute (0 = unlimited)")]
    max_requests_per_minute: usize,
}

/// Contains information about the state of balancebeam (e.g. what servers we are currently proxying
/// to, what servers have failed, rate limiting counts, etc.)
///
/// You should add fields to this struct in later milestones.
struct ProxyState {
    /// How frequently we check whether upstream servers are alive (Milestone 4)
    #[allow(dead_code)]
    active_health_check_interval: usize,
    /// Where we should send requests when doing active health checks (Milestone 4)
    #[allow(dead_code)]
    active_health_check_path: String,
    /// Maximum number of requests an individual IP can make in a minute (Milestone 5)
    #[allow(dead_code)]
    max_requests_per_minute: usize,
    /// Addresses of servers that we are proxying to
    upstream_addresses: Vec<String>,
    /// Indices of upstream_addresses that are alive.
    alive_indices: BTreeSet<usize>,
    /// request count info for each client. IP -> (curr_window_timestamp, prev_counter, cur_counter)
    request_limit_table: HashMap<String, (u64, u32, u32)>
}

#[tokio::main]
async fn main() {
    // Initialize the logging library. You can print log messages using the `log` macros:
    // https://docs.rs/log/0.4.8/log/ You are welcome to continue using print! statements; this
    // just looks a little prettier.
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "debug");
    }
    pretty_env_logger::init();

    // Parse the command line arguments passed to this program
    let options = CmdOptions::parse();
    if options.upstream.len() < 1 {
        log::error!("At least one upstream server must be specified using the --upstream option.");
        std::process::exit(1);
    }

    // Start listening for connections
    let listener = match TcpListener::bind(&options.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            log::error!("Could not bind to {}: {}", options.bind, err);
            std::process::exit(1);
        }
    };
    log::info!("Listening for requests on {}", options.bind);

    // Handle incoming connections
    let upstream_num = options.upstream.len();
    let health_check_interval = options.active_health_check_interval;
    let state = ProxyState {
        upstream_addresses: options.upstream,
        active_health_check_interval: options.active_health_check_interval,
        active_health_check_path: options.active_health_check_path,
        max_requests_per_minute: options.max_requests_per_minute,
        alive_indices: BTreeSet::from_iter(0..upstream_num),
        request_limit_table: HashMap::new(),
    };

    let state_lock = Arc::new(RwLock::new(state));
    let state_lock_c = state_lock.clone();
    let mut interval = time::interval(Duration::from_secs(health_check_interval as u64));

    // health checker
    tokio::task::spawn(async move {
        interval.tick().await;
        loop {
            interval.tick().await;
            {
                let mut state = state_lock_c.write().await;
                let addresses = state.upstream_addresses.clone();
                for (index, server) in addresses.iter().enumerate() {
                    match active_health_check(&server, &state.active_health_check_path).await {
                        Ok(()) => { state.alive_indices.insert(index); }
                        Err(_) => {
                            state.alive_indices.remove(&index);
                            log::warn!("Detected upstream server {} down", server);
                        }
                    }
                }
                // in case interval changes.
                if state.active_health_check_interval != interval.period().as_secs() as usize {
                    interval = time::interval(Duration::from_secs(state.active_health_check_interval as u64));
                    interval.tick().await;
                }
            }
        }
    });

    // request limit patrol
    let state_lock_c = state_lock.clone();
    if state_lock_c.read().await.max_requests_per_minute != 0 {
        let mut interval = time::interval(Duration::from_secs(60));     // update per minute
        tokio::task::spawn(async move {
            interval.tick().await;
            loop {
                interval.tick().await;
                let mut state = state_lock_c.write().await;
                state.request_limit_table.values_mut().for_each(|x| {
                    *x = (timestamp!(), x.2, 0);
                });
            }
        });
    }

    loop {
        if let Ok((mut client_conn, _)) = listener.accept().await {
            // Handle the connection!
            let state_lock_c = state_lock.clone();
            let upstream_conn = match connect_to_upstream(&state_lock_c).await {
                Ok(stream) => stream,
                Err(_error) => {
                    let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                    send_response(&mut client_conn, &response).await;
                    continue;
                }
            };
            tokio::task::spawn(async move {
                handle_connection(client_conn, upstream_conn, &state_lock_c).await;
            });
        }
    }
}


async fn active_health_check(upstream_address: &String, path: &String) -> Result<(), std::io::Error> {
    let mut upstream_conn = TcpStream::connect(upstream_address).await?;
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(path)
        .header("Host", upstream_conn.peer_addr().unwrap().ip().to_string())
        .body(Vec::new())
        .unwrap();
    request::write_to_stream(&request, &mut upstream_conn).await?;
    if let Ok(response) = response::read_from_stream(&mut upstream_conn, request.method()).await {
        if response.status() != StatusCode::OK {
            return Err(std::io::Error::from(ErrorKind::NotConnected));
        }
    }
    else { return Err(std::io::Error::from(ErrorKind::NotConnected)); }
    Ok(())
}


async fn connect_to_upstream(state_lock: &RwLock<ProxyState>) -> Result<TcpStream, std::io::Error> {
    let mut rng = rand::rngs::StdRng::from_entropy();
    loop {
        let upstream_ip;
        let upstream_idx;
        {
            // scope for read lock.
            let state = state_lock.read().await;
            upstream_idx = *state.alive_indices
                .iter().choose(&mut rng)
                .ok_or(std::io::Error::from(ErrorKind::NotConnected))?;
            upstream_ip = &state.upstream_addresses[upstream_idx];
            if let Ok(stream) = TcpStream::connect(upstream_ip).await {
                return Ok(stream);
            }
        }

        {
            // scope for write lock.
            let mut state = state_lock.write().await;
            state.alive_indices.remove(&upstream_idx);
        }
    }
}

async fn send_response(client_conn: &mut TcpStream, response: &http::Response<Vec<u8>>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("{} <- {}", client_ip, response::format_response_line(&response));
    if let Err(error) = response::write_to_stream(&response, client_conn).await {
        log::warn!("Failed to send response to client: {}", error);
        return;
    }
}

async fn handle_connection(mut client_conn: TcpStream, mut upstream_conn: TcpStream, state_lock: &RwLock<ProxyState>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("Connection received from {}", client_ip.clone());

    if state_lock.read().await.max_requests_per_minute != 0 {
        let mut state = state_lock.write().await;
        state.request_limit_table.entry(client_ip.clone()).or_insert((timestamp!(), 0, 0));
    }

    let upstream_ip = upstream_conn.peer_addr().unwrap().ip().to_string();

    // The client may now send us one or more requests. Keep trying to read requests until the
    // client hangs up or we get an error.
    loop {
        // Read a request from the client
        let mut request = match request::read_from_stream(&mut client_conn).await {
            Ok(request) => request,
            // Handle case where client closed connection and is no longer sending requests
            Err(request::Error::IncompleteRequest(0)) => {
                log::debug!("Client finished sending requests. Shutting down connection");
                return;
            }
            // Handle I/O error in reading from the client
            Err(request::Error::ConnectionError(io_err)) => {
                log::info!("Error reading request from client stream: {}", io_err);
                return;
            }
            Err(error) => {
                log::debug!("Error parsing request: {:?}", error);
                let response = response::make_http_error(match error {
                    request::Error::IncompleteRequest(_)
                    | request::Error::MalformedRequest(_)
                    | request::Error::InvalidContentLength
                    | request::Error::ContentLengthMismatch => http::StatusCode::BAD_REQUEST,
                    request::Error::RequestBodyTooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
                    request::Error::ConnectionError(_) => http::StatusCode::SERVICE_UNAVAILABLE,
                });
                send_response(&mut client_conn, &response).await;
                continue;
            }
        };

        // rate limiting
        let limit = state_lock.read().await.max_requests_per_minute;
        if limit != 0 {
            let mut state = state_lock.write().await;
            let limit_info = state.request_limit_table.get_mut(&client_ip).expect("Broken Hashmap.");
            let percent_through = (timestamp!() - limit_info.0) as f64 / 60.0;
            if (1.0 - percent_through) * limit_info.1 as f64 + (limit_info.2 as f64) < limit as f64 {
                limit_info.2 += 1;
            } else {
                drop(state);
                // make response
                let response = response::make_http_error(http::StatusCode::TOO_MANY_REQUESTS);
                send_response(&mut client_conn, &response).await;
                continue;
            }
        }

        log::info!(
            "{} -> {}: {}",
            client_ip,
            upstream_ip,
            request::format_request_line(&request)
        );

        // Add X-Forwarded-For header so that the upstream server knows the client's IP address.
        // (We're the ones connecting directly to the upstream server, so without this header, the
        // upstream server will only know our IP, not the client's.)
        request::extend_header_value(&mut request, "x-forwarded-for", &client_ip);

        // Forward the request to the server
        if let Err(error) = request::write_to_stream(&request, &mut upstream_conn).await {
            log::error!("Failed to send request to upstream {}: {}", upstream_ip, error);
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
        log::debug!("Forwarded request to server");

        // Read the server's response
        let response = match response::read_from_stream(&mut upstream_conn, request.method()).await {
            Ok(response) => response,
            Err(error) => {
                log::error!("Error reading response from server: {:?}", error);
                let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                send_response(&mut client_conn, &response).await;
                return;
            }
        };
        // Forward the response to the client
        send_response(&mut client_conn, &response).await;
        log::debug!("Forwarded response to client");
    }
}
