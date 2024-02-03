# Balancebeam
A simple network load balancer written in Rust, one of the Stanford CS110L projects.  

CS110L Assignment handouts are available [here](https://reberhardt.com/cs110l/spring-2020/).  

Adapt crates of newer versions according to [here](https://github.com/fung-hwang/CS110L-2020spr/tree/main/proj-2).  

**Include starter code from [here](https://github.com/reberhardt7/cs110l-spr-2020-starter-code)**. Most of the HTTP request/response handling code is provided by the starter code.

**Full development process** can be found [here](https://github.com/csBenClarkson/cs110l-spr-2020/tree/main/proj-2). It is separated due to bad project management 😭️.


# Description
This is a network load balancer written in Rust, forwarding requests from clients to upstream servers, and sending back responses to clients.

## Algorithms
1. Balancebeam **randomly** select an upstream server for connection.  
2. Balancebeam use the **sliding window** algorithm to implement rate limiting (if enabled). 

## Note
Balancebeam add header `x-forwarded-for` to requests so that the addresses of clients can be obtained by servers.

## Features
- **Asynchronous I/O**. [tokio](https://tokio.rs/) is used to implement asynchronous I/O in Rust. tokio's `async/await` syntax provide ease of development and great readability without compromising performance.

- **Fast** and achieve **high concurrency**. Asynchronous I/O and `Future` concept in Rust provide fast processing and minimal switching overhead between enormous number of network connections, increasing scalabilibty.

- **Failover infrastructure**. Balancebeam is equipped with passive and active health check on upstream servers so that the system still works even though some upstream servers down.

- **Rate limiting**. Balancebeam supports rate limiting on IP addresses. A restriction of maximum requests from an IP address per minute can be imposed.


# Usage
Command line arguments:  

- `-b, --bind <BIND>`: address and port binding for Balancebeam to listen on. Default: `0.0.0.0:1100`.  

- `-u, --upstream <UPSTREAM>`: upstream server addresses and port. Example: `cargo run -u 127.0.0.1:8080 -u 127.0.0.1:8081`

- `--active-health-check-path <ACTIVE_HEALTH_CHECK_PATH>`: The path of upstream servers to send requests to, in order to perform active health check.

- `--active-health-check-interval <ACTIVE_HEALTH_CHECK_INTERVAL>`: The frequency in seconds that an active health check is performed. Default: `10` seconds. 

- `--max-requests-per-minute <MAX_REQUESTS_PER_MINUTE>`: Maximum number of requests to accept per IP per minute (0 = unlimited). Default: `0`.

- `-h, --help`: Print help.


In `balancebeam/` directory,
```
cargo run [OPTIONS]
```
or in build directory,
```
<compiled-program> [OPTIONS]
```

# Benchmark
Check [here](https://web.stanford.edu/class/cs110l/assignments/project-2-2022/) to get **Performance Testing** suggestions. Here a simple benchmark method is provided.

1. Download [wrk2](https://github.com/giltene/wrk2), a HTTP benchmarking tool. Compile the program.
    ```
    git clone https://github.com/giltene/wrk2.git
    cd wrk2
    make
    ```
    the executable `wrk2` should be in the root directory of the respository.

1. Run some upstream servers. Here [actix](https://actix.rs/) is used. In `benchmark/`, codes is provided for a server on `localhost` that echos "hello world". Open serveral terminal and run `cargo run -r -- <PORT>` with different ports.

1. Run the load balancer Balancebeam with upstream servers that are started in pervious step.
    ```
    cargo run -r -- -u <SERVER 1> -u <SERVER 2> ...
    ```

1. Run `wrk2` on the load balancer. Please refer to README.md in [wrk2](https://github.com/giltene/wrk2) to get detailed usage.  

    Here is an example: in `wrk2/`, run
    ```
    ./wrk -t4 -c100 -d30s -R1000 http://127.0.0.1:1100/
    ```
    This command sends 1000 requests per second to `http://127.0.0.1:1100/` (where Balancebeam listens on) for 30 seconds, with 4 threads and 100 connections.  

    The result will be printed. Check out the metrics to assess the performance.



