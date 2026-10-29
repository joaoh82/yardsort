//! A stand-in for Fly.io's proxy in front of several instances of the server on this machine, for
//! the relay's smoke test with more than one ([27 §17]). Requests go to each instance in turn, and
//! an answer saying `fly-replay` sends the request on, as Fly.io's proxy does. It prints where each
//! request went: its method and path, never its query or headers.
//!
//! ```text
//! cargo run -p yardsort-server --example replay_proxy -- LISTEN NAME=ADDRESS NAME=ADDRESS …
//! cargo run -p yardsort-server --example replay_proxy -- 127.0.0.1:8787 a=127.0.0.1:8788 b=127.0.0.1:8789
//! ```
//!
//! Each instance runs with `YARDSORT_ROUTING=fly-replay`, `YARDSORT_INSTANCE=NAME` and
//! `YARDSORT_LISTEN=ADDRESS`, and its public URL is the proxy's.
//!
//! [27 §17]: ../../../docs/design/27-remote-r2.md#17--smoke-tests

// The tests' own stand-in, so the smoke test runs what they check. Some of it is for tests only.
#[allow(dead_code)]
#[path = "../tests/common/replay.rs"]
mod replay;

use std::net::SocketAddr;

const USAGE: &str = "usage: replay_proxy LISTEN NAME=ADDRESS NAME=ADDRESS …";

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let listen: SocketAddr = args
        .next()
        .ok_or(USAGE)?
        .parse()
        .map_err(|e| format!("LISTEN: {e}\n{USAGE}"))?;
    let instances = args
        .map(|arg| {
            let (name, addr) = arg.split_once('=').ok_or(USAGE)?;
            let addr = addr.parse().map_err(|e| format!("{arg}: {e}\n{USAGE}"))?;
            Ok((name.to_owned(), addr))
        })
        .collect::<Result<Vec<(String, SocketAddr)>, String>>()?;
    if instances.is_empty() {
        return Err(USAGE.into());
    }
    let proxy = replay::ReplayProxy::bind(listen, instances, true)
        .await
        .map_err(|e| format!("listening on {listen}: {e}"))?;
    println!(
        "In front of the instances at {}. Ctrl-C stops it.",
        proxy.base()
    );
    let _ = tokio::signal::ctrl_c().await;
    Ok(())
}
