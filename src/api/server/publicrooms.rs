use axum_client_ip::SecureClientIp;
use ruma::{
	api::{
		client::error::ErrorKind,
		federation::directory::{get_public_rooms, get_public_rooms_filtered},
	},
	directory::Filter,
};

use crate::{services, Error, Result, Ruma};

/// # `POST /_matrix/federation/v1/publicRooms`
///
/// Lists the public rooms on this server.
#[tracing::instrument(skip_all, fields(client_ip = %ip))]
pub(crate) async fn get_public_rooms_filtered_route(
	SecureClientIp(ip): SecureClientIp, body: Ruma<get_public_rooms_filtered::v1::Request>,
) -> Result<get_public_rooms_filtered::v1::Response> {
	if !services()
		.globals
		.allow_public_room_directory_over_federation()
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Room directory is not public"));
	}

	let response = crate::client::get_public_rooms_filtered_helper(
		None,
		body.limit,
		body.since.as_deref(),
		&body.filter,
		&body.room_network,
	)
	.await
	.map_err(|_| Error::BadRequest(ErrorKind::Unknown, "Failed to return this server's public room list."))?;

	Ok(get_public_rooms_filtered::v1::Response {
		chunk: response.chunk,
		prev_batch: response.prev_batch,
		next_batch: response.next_batch,
		total_room_count_estimate: response.total_room_count_estimate,
	})
}

/// # `GET /_matrix/federation/v1/publicRooms`
///
/// Lists the public rooms on this server.
#[tracing::instrument(skip_all, fields(client_ip = %ip))]
pub(crate) async fn get_public_rooms_route(
	SecureClientIp(ip): SecureClientIp, body: Ruma<get_public_rooms::v1::Request>,
) -> Result<get_public_rooms::v1::Response> {
	if !services()
		.globals
		.allow_public_room_directory_over_federation()
	{
		return Err(Error::BadRequest(ErrorKind::forbidden(), "Room directory is not public"));
	}

	let response = crate::client::get_public_rooms_filtered_helper(
		None,
		body.limit,
		body.since.as_deref(),
		&Filter::default(),
		&body.room_network,
	)
	.await
	.map_err(|_| Error::BadRequest(ErrorKind::Unknown, "Failed to return this server's public room list."))?;

	Ok(get_public_rooms::v1::Response {
		chunk: response.chunk,
		prev_batch: response.prev_batch,
		next_batch: response.next_batch,
		total_room_count_estimate: response.total_room_count_estimate,
	})
}
