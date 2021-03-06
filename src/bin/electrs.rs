extern crate electrs;

extern crate error_chain;
#[macro_use]
extern crate log;

use error_chain::ChainedError;
use std::process;
use std::time::Duration;

use electrs::{
    app::App, bulk, config::Config, daemon::Daemon, errors::*, index::Index, metrics::Metrics,
    query::Query, rpc::RPC, signal::Waiter, store::DBStore,
};

fn run_server(config: &Config) -> Result<()> {
    let signal = Waiter::new();
    let metrics = Metrics::new(config.monitoring_addr);
    metrics.start();

    let daemon = Daemon::new(
        &config.daemon_dir,
        config.daemon_rpc_addr,
        config.cookie_getter(),
        config.network_type,
        signal.clone(),
        &metrics,
    )?;
    // Perform initial indexing from local blk*.dat block files.
    let store = DBStore::open(&config.db_path);
    let index = Index::load(&store, &daemon, &metrics, config.index_batch_size)?;
    let store = if config.skip_bulk_import {
        index.update(&store, &signal)?; // slower: uses JSONRPC for fetching blocks
        bulk::full_compaction(store)
    } else {
        bulk::index(&daemon, &metrics, store) // faster, but uses more memory
    }?;

    let app = App::new(store, index, daemon)?;
    let query = Query::new(app.clone(), &metrics);

    let mut server = None; // Electrum RPC server
    loop {
        app.update(&signal)?;
        query.update_mempool()?;
        server
            .get_or_insert_with(|| RPC::start(config.electrum_rpc_addr, query.clone(), &metrics))
            .notify(); // update subscribed clients
        if let Err(err) = signal.wait(Duration::from_secs(5)) {
            info!("stopping server: {}", err);
            break;
        }
    }
    Ok(())
}

fn main() {
    let config = Config::from_args();
    if let Err(e) = run_server(&config) {
        error!("server failed: {}", e.display_chain());
        process::exit(1);
    }
}
