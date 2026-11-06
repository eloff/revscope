use std::net::IpAddr;
use std::sync::Arc;

use compact_str::CompactString;

use super::batch::{BatchData, MAX_ORPHAN_EVENTS};
use super::constraints::{
    validate_engagement_constraints, validate_event_metadata, validate_landing_context_constraints,
    validate_metadata_json_constraints,
};
use super::error::PersistError;
use super::failure::{IngestRowFailure, constraint_failure, unknown_page_failure, wire_json};
use super::flush::schedule_threshold_flush;
use super::lookup::resolve_enqueue_pageview_ref;
use super::map::IngestBatch;
use super::normalize::{
    normalized_duration_seconds, normalized_metadata_object, normalized_screen_dimensions,
    positive_timestamp_id, required_non_empty, resolve_language,
};
use super::pageview::enqueue_pageview;
use super::parse::ValidatedPayload;
use super::rows::{
    CrawlerViewKey, EngagementRow, EventRow, ForgetRow, LandingContextRow, MetadataUpdateRow,
    OrphanEvent,
};
use super::session::{SessionCookie, apply_session_from_cookie, build_user_hash};
use super::timestamp::{INGEST_LATENCY_ALLOWANCE_NANOS, NANOS_PER_MILLI, assign_event_timestamp};
use super::user_agent::normalized_user_agent;
use super::wire::{HitKind, JsonField, TrackRequest};
use crate::db::{ClientPageId, ProjectConnection};
use crate::http::ParsedRequest;
use crate::server_context::ServerContext;
use crate::util::{nanoseconds_since_epoch, non_empty};

#[derive(Debug, Default)]
pub(super) struct PersistOutcome {
    pub(super) visitor_timestamp_id: Option<i64>,
    pub(super) storage_allowed: bool,
    pub(super) session_cookie: Option<SessionCookie>,
}

/// Enqueue validated pageview/event/engagement/metadata items into the project's batch,
/// resolving just enough to answer the tracker synchronously. The actual table
/// writes happen later in a single flush transaction (on a background task when
/// the batch crosses [`BATCH_FLUSH_THRESHOLD`](super::flush::BATCH_FLUSH_THRESHOLD), otherwise on the timer).
pub(super) fn enqueue_items(
    parsed: &ParsedRequest<'_>,
    client_ip: Option<IpAddr>,
    cf_country: Option<&str>,
    server_context: Arc<ServerContext>,
    project_id: &str,
    project_name: &str,
    items: Vec<ValidatedPayload>,
) -> Result<(PersistOutcome, Vec<IngestRowFailure>), PersistError> {
    let project_key = crate::db::parse_project_id(project_id)?;
    let mut outcome = PersistOutcome::default();
    let mut constraint_failures = Vec::new();
    let mut read_conn: Option<ProjectConnection> = None;
    let mut guard = server_context.ingest_map.get(project_key);

    for mut item in items {
        apply_session_from_cookie(&mut item.wire, parsed.cookie, project_name);
        enqueue_item(
            &server_context,
            project_id,
            &mut guard,
            &mut read_conn,
            parsed,
            client_ip,
            cf_country,
            item,
            &mut outcome,
            &mut constraint_failures,
        )?;
    }

    drop(read_conn);
    schedule_threshold_flush(&server_context, guard);

    Ok((outcome, constraint_failures))
}

#[allow(clippy::too_many_arguments)]
fn enqueue_item(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    parsed: &ParsedRequest<'_>,
    client_ip: Option<IpAddr>,
    cf_country: Option<&str>,
    mut item: ValidatedPayload,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let client_age_ms = item.client_age_ms;
    match item.hit_kind {
        HitKind::Pageview => enqueue_pageview(
            server_context,
            project_id,
            batch,
            read_conn,
            parsed,
            client_ip,
            cf_country,
            &mut item.wire,
            client_age_ms,
            outcome,
            constraint_failures,
        ),
        HitKind::Append => enqueue_engagement(
            server_context,
            project_id,
            batch,
            read_conn,
            &item.wire,
            outcome,
            constraint_failures,
        ),
        HitKind::Event => enqueue_event(
            server_context,
            project_id,
            batch,
            read_conn,
            client_ip,
            &item.wire,
            client_age_ms,
            outcome,
            constraint_failures,
        ),
        HitKind::Metadata => enqueue_metadata(
            server_context,
            project_id,
            batch,
            read_conn,
            &item.wire,
            outcome,
            constraint_failures,
        ),
        HitKind::Context => enqueue_context(
            server_context,
            project_id,
            batch,
            read_conn,
            parsed,
            &item.wire,
            outcome,
            constraint_failures,
        ),
        HitKind::Forget => enqueue_forget(parsed, client_ip, batch, &item.wire),
        HitKind::Crawler => enqueue_crawler(parsed, client_ip, batch, &item.wire, client_age_ms),
        HitKind::Error => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn enqueue_event(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    client_ip: Option<IpAddr>,
    wire: &TrackRequest,
    client_age_ms: i64,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let event_name = CompactString::new(required_non_empty(wire.event.as_deref())?);
    let metadata = wire
        .metadata
        .as_ref()
        .and_then(JsonField::to_normalized_json);
    if let Err(error_text) = validate_event_metadata(metadata.as_deref()) {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }
    let received_nanos = i64::try_from(nanoseconds_since_epoch())?;

    let Some((page_timestamp_id, visitor_timestamp_id)) =
        resolve_enqueue_pageview_ref(server_context, project_id, batch, read_conn, wire)?
    else {
        // Soft-drop: one orphan event must not 400 the rest of a client batch.
        let client_page_id = non_empty(wire.page_id.as_deref())
            .map(ClientPageId::from_hex)
            .transpose()
            .map_err(|_| PersistError::InvalidPayload)?;
        match client_page_id {
            Some(client_page_id) if batch.orphan_events.len() < MAX_ORPHAN_EVENTS => {
                batch.orphan_events.push(OrphanEvent {
                    client_page_id,
                    event_name,
                    metadata,
                    received_nanos,
                    client_age_ms,
                    wire_json: wire_json(wire),
                    client_ip,
                });
            }
            _ => constraint_failures.push(unknown_page_failure(wire_json(wire))),
        }
        return Ok(());
    };
    let timestamp_id = assign_event_timestamp(
        server_context,
        project_id,
        batch,
        read_conn,
        received_nanos,
        client_age_ms,
        page_timestamp_id,
    )?;

    let data: &mut BatchData = &mut *batch;
    data.events.push(EventRow {
        timestamp_id,
        event_name,
        page_timestamp_id,
        metadata,
    });
    data.total_events += 1;

    outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
    Ok(())
}

/// Queue the orphan events that were waiting for this pageview.
pub(super) fn adopt_orphan_events(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    client_page_id: &ClientPageId,
    page_timestamp_id: i64,
) -> Result<(), PersistError> {
    if batch.orphan_events.is_empty() {
        return Ok(());
    }
    let (adopted, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut batch.orphan_events)
        .into_iter()
        .partition(|orphan| orphan.client_page_id == *client_page_id);
    batch.orphan_events = waiting;
    for orphan in adopted {
        let timestamp_id = assign_event_timestamp(
            server_context,
            project_id,
            batch,
            read_conn,
            orphan.received_nanos,
            orphan.client_age_ms,
            page_timestamp_id,
        )?;
        let data: &mut BatchData = &mut *batch;
        data.events.push(EventRow {
            timestamp_id,
            event_name: orphan.event_name,
            page_timestamp_id,
            metadata: orphan.metadata,
        });
        data.total_events += 1;
    }
    Ok(())
}

fn enqueue_engagement(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    wire: &TrackRequest,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let Some((page_timestamp_id, visitor_timestamp_id)) =
        resolve_enqueue_pageview_ref(server_context, project_id, batch, read_conn, wire)?
    else {
        return Err(PersistError::InvalidPayload);
    };

    let duration_increment = normalized_duration_seconds(wire.duration).unwrap_or_default();
    let scrolled_percent = wire.scrolled.map(|value| value.clamp(0, 100));

    let engagement = EngagementRow {
        page_timestamp_id,
        duration_increment,
        scrolled_percent,
    };
    if let Err(error_text) = validate_engagement_constraints(&engagement) {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }

    let data: &mut BatchData = &mut *batch;
    data.engagements.push(engagement);
    data.total_events += 1;

    outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
    Ok(())
}

fn enqueue_metadata(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    wire: &TrackRequest,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let Some((page_timestamp_id, visitor_timestamp_id)) =
        resolve_enqueue_pageview_ref(server_context, project_id, batch, read_conn, wire)?
    else {
        return Err(PersistError::InvalidPayload);
    };
    let metadata_json = normalized_metadata_object(wire.metadata.as_ref())?;
    if let Err(error_text) = validate_metadata_json_constraints(&metadata_json) {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }

    let data: &mut BatchData = &mut *batch;
    data.metadata_updates.push(MetadataUpdateRow {
        batch_order: data.total_events,
        page_timestamp_id,
        visitor_timestamp_id,
        metadata_json,
    });
    data.total_events += 1;

    outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
    Ok(())
}

/// Queue the terminal-device context follow-up hit. The tracker sends this only
/// after an ingest response set `storage_allowed`, or after the visitor
/// explicitly granted consent, so reaching here means collection is allowed.
#[allow(clippy::too_many_arguments)]
fn enqueue_context(
    server_context: &ServerContext,
    project_id: &str,
    batch: &mut IngestBatch,
    read_conn: &mut Option<ProjectConnection>,
    parsed: &ParsedRequest<'_>,
    wire: &TrackRequest,
    outcome: &mut PersistOutcome,
    constraint_failures: &mut Vec<IngestRowFailure>,
) -> Result<(), PersistError> {
    let Some((_, visitor_timestamp_id)) =
        resolve_enqueue_pageview_ref(server_context, project_id, batch, read_conn, wire)?
    else {
        return Err(PersistError::InvalidPayload);
    };

    let context = LandingContextRow {
        visitor_timestamp_id,
        timezone: wire.timezone.clone(),
        language: resolve_language(wire, parsed),
        screen_dimensions: normalized_screen_dimensions(wire),
        touch: wire.touch.unwrap_or(false),
    };
    if let Err(error_text) = validate_landing_context_constraints(&context) {
        constraint_failures.push(constraint_failure(wire, error_text));
        return Ok(());
    }

    let data: &mut BatchData = &mut *batch;
    data.context_updates.push(context);
    data.total_events += 1;

    outcome.visitor_timestamp_id = Some(visitor_timestamp_id);
    Ok(())
}

fn enqueue_crawler(
    parsed: &ParsedRequest<'_>,
    client_ip: Option<IpAddr>,
    batch: &mut IngestBatch,
    wire: &TrackRequest,
    client_age_ms: i64,
) -> Result<(), PersistError> {
    let hostname = CompactString::new(required_non_empty(wire.hostname.as_deref())?);
    let path = CompactString::new(required_non_empty(wire.path.as_deref())?);
    let Some(useragent) = normalized_user_agent(parsed.user_agent) else {
        return Ok(());
    };
    record_crawler_view(
        batch,
        client_age_ms,
        &useragent,
        &hostname,
        &path,
        client_ip,
    );
    Ok(())
}

pub(super) fn record_crawler_view(
    batch: &mut IngestBatch,
    client_age_ms: i64,
    useragent: &str,
    hostname: &str,
    path: &str,
    client_ip: Option<IpAddr>,
) {
    let hour = crawler_view_hour(client_age_ms);
    let verified = crate::crawlers::verify_operator(useragent, client_ip);
    let ip = if verified {
        None
    } else {
        client_ip.map(|ip| CompactString::new(ip.to_string()))
    };
    let data: &mut BatchData = &mut *batch;
    let key = CrawlerViewKey {
        hour,
        useragent: CompactString::new(useragent),
        ip,
        hostname: CompactString::new(hostname),
        path: CompactString::new(path),
        verified,
    };
    *data.crawler_views.entry(key).or_insert(0) += 1;
    data.total_events += 1;
}

fn crawler_view_hour(client_age_ms: i64) -> i64 {
    use crate::util::NANOS_PER_SECOND;
    const HOUR_SECONDS: i64 = 3600;
    let event_nanos = i64::try_from(nanoseconds_since_epoch())
        .unwrap_or(i64::MAX)
        .saturating_sub(client_age_ms.saturating_mul(NANOS_PER_MILLI))
        .saturating_sub(INGEST_LATENCY_ALLOWANCE_NANOS);
    let event_secs = event_nanos.div_euclid(NANOS_PER_SECOND);
    event_secs - event_secs.rem_euclid(HOUR_SECONDS)
}

fn enqueue_forget(
    parsed: &ParsedRequest<'_>,
    client_ip: Option<IpAddr>,
    batch: &mut IngestBatch,
    wire: &TrackRequest,
) -> Result<(), PersistError> {
    let client_page_id = non_empty(wire.page_id.as_deref())
        .map(ClientPageId::from_hex)
        .transpose()
        .map_err(|_| PersistError::InvalidPayload)?;
    let session_id = positive_timestamp_id(wire.session_id)?;
    let client_ip_text = client_ip.map(|ip| ip.to_string());

    let data: &mut BatchData = &mut *batch;
    data.forgets.push(ForgetRow {
        user_hash: build_user_hash(parsed.user_agent, client_ip_text.as_deref()),
        client_page_id,
        session_id,
    });
    data.total_events += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::batch::ORPHAN_EVENT_GRACE_NANOS;
    use crate::ingest::test_support::{sample_orphan_event, sample_payload};

    #[test]
    fn pageview_adopts_the_orphan_events_waiting_for_its_page_id() {
        let server_context =
            crate::server_context::ServerContext::new_in_memory("orphan-event-adopted");
        let page_id = ClientPageId::from_hex("00112233445566778899aabb").expect("page id");
        // Received well before the pageview id it ends up attached to.
        let page_timestamp_id = 10 * ORPHAN_EVENT_GRACE_NANOS;
        let mut batch = IngestBatch::new(0, 0);
        batch.db_max_event_ts = Some(0);
        batch
            .orphan_events
            .push(sample_orphan_event(page_timestamp_id - 1_000));
        let mut other = sample_orphan_event(page_timestamp_id);
        other.client_page_id = ClientPageId::from_hex("ffeeddccbbaa998877665544").expect("page id");
        batch.orphan_events.push(other);

        adopt_orphan_events(
            &server_context,
            "1",
            &mut batch,
            &mut None,
            &page_id,
            page_timestamp_id,
        )
        .expect("adopt");

        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].event_name, "signup");
        assert_eq!(batch.events[0].page_timestamp_id, page_timestamp_id);
        assert!(batch.events[0].timestamp_id > page_timestamp_id);
        assert_eq!(batch.total_events, 1);
        assert_eq!(batch.orphan_events.len(), 1);
    }

    #[test]
    fn event_without_a_page_id_is_dropped_immediately() {
        let server_context =
            crate::server_context::ServerContext::new_in_memory("orphan-event-no-pg");
        let mut batch = IngestBatch::new(0, 0);
        let mut wire = sample_payload();
        wire.event = Some(CompactString::new("signup"));
        wire.page_id = None;
        let mut constraint_failures = Vec::new();
        enqueue_event(
            &server_context,
            "1",
            &mut batch,
            &mut None,
            None,
            &wire,
            0,
            &mut PersistOutcome::default(),
            &mut constraint_failures,
        )
        .expect("enqueue event");

        assert_eq!(constraint_failures.len(), 1);
        assert_eq!(constraint_failures[0].error_type, "unknown_page");
        assert!(batch.is_empty());
    }
}
