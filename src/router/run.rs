use std::{sync::Arc, time::Duration};

use axum_server::Handle as ServerHandle;
use tokio::sync::broadcast::{self, Sender};
use tracing::{debug, error, info};

extern crate conduit_admin as admin;
extern crate conduit_core as conduit;
extern crate conduit_service as service;

use std::sync::atomic::Ordering;

use conduit::{debug_info, trace, Error, Result, Server};
use service::services;

use crate::{layers, serve};

/// Main loop base
#[tracing::instrument(skip_all)]
#[allow(clippy::let_underscore_must_use)] // various of these are intended
pub(crate) async fn run(server: Arc<Server>) -> Result<(), Error> {
	let app = layers::build(&server)?;

	// Install the admin room callback here for now
	_ = services().admin.handle.lock().await.insert(admin::handle);

	// Setup shutdown/signal handling
	server.stopping.store(false, Ordering::Release);
	let handle = ServerHandle::new();
	let (tx, _) = broadcast::channel::<()>(1);
	let sigs = server
		.runtime()
		.spawn(signal(server.clone(), tx.clone(), handle.clone()));

	// Serve clients
	let res = serve::serve(&server, app, handle, tx.subscribe()).await;

	// Join the signal handler before we leave.
	sigs.abort();
	_ = sigs.await;

	// Remove the admin room callback
	_ = services().admin.handle.lock().await.take();

	debug_info!("Finished");
	res
}

/// Async initializations
#[tracing::instrument(skip_all)]
pub(crate) async fn start(server: Arc<Server>) -> Result<(), Error> {
	debug!("Starting...");

	service::init(&server).await?;
	services().start().await?;

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Ready]).expect("failed to notify systemd of ready state");

	debug!("Started");
	Ok(())
}

/// Async destructions
#[tracing::instrument(skip_all)]
pub(crate) async fn stop(_server: Arc<Server>) -> Result<(), Error> {
	debug!("Shutting down...");

	// Wait for all completions before dropping or we'll lose them to the module
	// unload and explode.
	services().stop().await;
	// Deactivate services(). Any further use will panic the caller.
	service::fini();

	debug!("Cleaning up...");

	#[cfg(feature = "systemd")]
	sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]).expect("failed to notify systemd of stopping state");

	info!("Shutdown complete.");
	Ok(())
}

#[tracing::instrument(skip_all)]
async fn signal(server: Arc<Server>, tx: Sender<()>, handle: axum_server::Handle) {
	let sig: &'static str = server
		.signal
		.subscribe()
		.recv()
		.await
		.expect("channel error");

	debug!("Received signal {}", sig);
	if sig == "SIGINT" {
		let reload = cfg!(unix) && cfg!(debug_assertions);
		server.reloading.store(reload, Ordering::Release);
	}

	server.stopping.store(true, Ordering::Release);
	if let Err(e) = tx.send(()) {
		error!("failed sending shutdown transaction to channel: {e}");
	}

	let pending = server.requests_spawn_active.load(Ordering::Relaxed);
	if pending > 0 {
		let timeout = Duration::from_secs(36);
		trace!(pending, ?timeout, "Notifying for graceful shutdown");
		handle.graceful_shutdown(Some(timeout));
	} else {
		debug!(pending, "Notifying for immediate shutdown");
		handle.shutdown();
	}
}
