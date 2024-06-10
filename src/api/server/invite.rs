use axum_client_ip::SecureClientIp;
use ruma::{
	api::{client::error::ErrorKind, federation::membership::create_invite},
	events::room::member::{MembershipState, RoomMemberEventContent},
	serde::JsonObject,
	CanonicalJsonValue, EventId, OwnedUserId,
};
use tracing::warn;

use crate::{
	service::server_is_ours,
	services,
	utils::{self},
	Error, PduEvent, Result, Ruma,
};

/// # `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}`
///
/// Invites a remote user to a room.
#[tracing::instrument(skip_all, fields(client_ip = %ip))]
pub(crate) async fn create_invite_route(
	SecureClientIp(ip): SecureClientIp, body: Ruma<create_invite::v2::Request>,
) -> Result<create_invite::v2::Response> {
	let origin = body.origin.as_ref().expect("server is authenticated");

	// ACL check origin
	services()
		.rooms
		.event_handler
		.acl_check(origin, &body.room_id)?;

	if !services()
		.globals
		.supported_room_versions()
		.contains(&body.room_version)
	{
		return Err(Error::BadRequest(
			ErrorKind::IncompatibleRoomVersion {
				room_version: body.room_version.clone(),
			},
			"Server does not support this room version.",
		));
	}

	if let Some(server) = body.room_id.server_name() {
		if services()
			.globals
			.config
			.forbidden_remote_server_names
			.contains(&server.to_owned())
		{
			return Err(Error::BadRequest(
				ErrorKind::forbidden(),
				"Server is banned on this homeserver.",
			));
		}
	}

	if services()
		.globals
		.config
		.forbidden_remote_server_names
		.contains(origin)
	{
		warn!(
			"Received federated/remote invite from banned server {origin} for room ID {}. Rejecting.",
			body.room_id
		);
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"Server is banned on this homeserver.",
		));
	}

	if let Some(via) = &body.via {
		if via.is_empty() {
			return Err(Error::BadRequest(ErrorKind::InvalidParam, "via field must not be empty."));
		}
	}

	let mut signed_event = utils::to_canonical_object(&body.event)
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invite event is invalid."))?;

	let invited_user: OwnedUserId = serde_json::from_value(
		signed_event
			.get("state_key")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event has no state_key property."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "state_key is not a user ID."))?;

	if !server_is_ours(invited_user.server_name()) {
		return Err(Error::BadRequest(
			ErrorKind::InvalidParam,
			"User does not belong to this homeserver.",
		));
	}

	// Make sure we're not ACL'ed from their room.
	services()
		.rooms
		.event_handler
		.acl_check(invited_user.server_name(), &body.room_id)?;

	ruma::signatures::hash_and_sign_event(
		services().globals.server_name().as_str(),
		services().globals.keypair(),
		&mut signed_event,
		&body.room_version,
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Failed to sign event."))?;

	// Generate event id
	let event_id = EventId::parse(format!(
		"${}",
		ruma::signatures::reference_hash(&signed_event, &body.room_version)
			.expect("ruma can calculate reference hashes")
	))
	.expect("ruma's reference hashes are valid event ids");

	// Add event_id back
	signed_event.insert("event_id".to_owned(), CanonicalJsonValue::String(event_id.to_string()));

	let sender: OwnedUserId = serde_json::from_value(
		signed_event
			.get("sender")
			.ok_or_else(|| Error::BadRequest(ErrorKind::InvalidParam, "Event had no sender property."))?
			.clone()
			.into(),
	)
	.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "sender is not a user ID."))?;

	if services().rooms.metadata.is_banned(&body.room_id)? && !services().users.is_admin(&invited_user)? {
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"This room is banned on this homeserver.",
		));
	}

	if services().globals.block_non_admin_invites() && !services().users.is_admin(&invited_user)? {
		return Err(Error::BadRequest(
			ErrorKind::forbidden(),
			"This server does not allow room invites.",
		));
	}

	let mut invite_state = body.invite_room_state.clone();

	let mut event: JsonObject = serde_json::from_str(body.event.get())
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid invite event bytes."))?;

	event.insert("event_id".to_owned(), "$placeholder".into());

	let pdu: PduEvent = serde_json::from_value(event.into())
		.map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Invalid invite event."))?;

	invite_state.push(pdu.to_stripped_state_event());

	// If we are active in the room, the remote server will notify us about the join
	// via /send
	if !services()
		.rooms
		.state_cache
		.server_in_room(services().globals.server_name(), &body.room_id)?
	{
		services().rooms.state_cache.update_membership(
			&body.room_id,
			&invited_user,
			RoomMemberEventContent::new(MembershipState::Invite),
			&sender,
			Some(invite_state),
			body.via.clone(),
			true,
		)?;
	}

	Ok(create_invite::v2::Response {
		event: PduEvent::convert_to_outgoing_federation_event(signed_event),
	})
}
