use std::net::IpAddr;

use fxhash::{FxHashMap, FxHashSet};
use serde::Serialize;

use super::failure::{IngestRowFailure, unknown_page_failure};
use super::flush::BATCH_FLUSH_THRESHOLD;
use super::rows::{
    CrawlerViewKey, EngagementRow, EventRow, ForgetRow, LandingContextRow, LandingPageView,
    MetadataUpdateRow, OrphanEvent, PageView, PageviewRef,
};
use super::timestamp::NANOS_PER_MILLI;
use crate::db::ClientPageId;

/// How long an event whose pageview has not arrived yet survives a flush. The
/// tracker can send an event while the pageview request it belongs to is still
/// in flight, so a flush that lands in between keeps the event for one more
/// interval instead of dropping it.
pub(super) const ORPHAN_EVENT_GRACE_NANOS: i64 = 10_000 * NANOS_PER_MILLI;

/// Most events a batch holds while they wait for their pageview. Past this,
/// further orphans are dropped as soon as they arrive.
pub(super) const MAX_ORPHAN_EVENTS: usize = BATCH_FLUSH_THRESHOLD;

/// The queued rows for one project plus the in-memory lookup maps that
/// supplement the database for rows that have not been flushed yet.
///
/// Serialized when a flush gives up on the batch, so every field carrying data
/// that is not also on disk has to round-trip losslessly (blobs and the 128-bit
/// user hash go out as hex rather than as JSON numbers). The lookup maps and id
/// floors below are skipped: they are indexes into, or bookkeeping derived from,
/// the row vectors, so they carry nothing the rows do not already say.
#[derive(Default, Serialize)]
pub(crate) struct BatchData {
    pub(crate) pageviews: Vec<PageView>,
    pub(super) landings: Vec<LandingPageView>,
    pub(super) events: Vec<EventRow>,
    pub(super) engagements: Vec<EngagementRow>,
    pub(super) metadata_updates: Vec<MetadataUpdateRow>,
    pub(super) context_updates: Vec<LandingContextRow>,
    pub(super) forgets: Vec<ForgetRow>,
    /// Events waiting for their pageview. Not counted in `total_events`, and kept
    /// across a flush that falls within their grace window.
    pub(super) orphan_events: Vec<OrphanEvent>,
    /// Hourly crawler hits keyed by (hour, useragent, hostname, path).
    #[serde(skip)]
    pub(super) crawler_views: FxHashMap<CrawlerViewKey, i64>,
    /// client page id (12-byte blob) -> queued pageview. Supplements
    /// `client_page_ids_map` for pageviews not yet flushed.
    #[serde(skip)]
    pub(super) pageview_idx_by_client_id: FxHashMap<ClientPageId, PageviewRef>,
    /// pageview timestamp id (a.k.a. server page id) -> queued pageview.
    #[serde(skip)]
    pub(super) pageview_idx_by_ts: FxHashMap<i64, PageviewRef>,
    /// user hash -> visitor_timestamp_id, supplementing `user_sessions` for
    /// cookieless attribution within the batch.
    #[serde(skip)]
    pub(super) visitor_by_user_hash: FxHashMap<u128, i64>,
    /// Highest timestamp id already written for this project, per type. Read
    /// lazily and dropped on flush so it always covers the rows on disk;
    /// backdated ids stay above it and therefore cannot collide with them.
    #[serde(skip)]
    pub(super) db_max_pageview_ts: Option<i64>,
    #[serde(skip)]
    pub(super) db_max_event_ts: Option<i64>,
    /// Timestamp ids handed out to the queued rows, per type. Backdating lets
    /// two hits land on the same nanosecond, so ids are checked against these
    /// before they are used as primary keys.
    #[serde(skip)]
    pub(super) queued_pageview_ts: FxHashSet<i64>,
    #[serde(skip)]
    pub(super) queued_event_ts: FxHashSet<i64>,
    /// Count of all queued hits.
    pub(super) total_events: usize,
}

impl BatchData {
    pub(super) fn is_empty(&self) -> bool {
        self.total_events == 0 && self.orphan_events.is_empty()
    }

    /// Queues `page` as a pageview hit would, without the lookups enqueue
    /// does first, for tests of the flush.
    #[cfg(test)]
    pub(super) fn queue_test_pageview(&mut self, page: PageView) {
        let index = PageviewRef::try_from(self.pageviews.len()).expect("pageview index");
        self.pageview_idx_by_ts.insert(page.timestamp_id, index);
        self.pageview_idx_by_client_id
            .insert(page.client_page_id.clone(), index);
        self.pageviews.push(page);
        self.total_events += 1;
    }

    /// Whether enough hits are queued that the batch should be flushed now
    /// rather than waiting for the timer.
    pub(super) fn at_flush_threshold(&self) -> bool {
        self.total_events >= BATCH_FLUSH_THRESHOLD
    }

    /// Whether any rows are queued to be written, ignoring orphan events.
    pub(crate) fn has_rows(&self) -> bool {
        self.total_events > 0
    }

    pub(super) fn has_orphan_events(&self) -> bool {
        !self.orphan_events.is_empty()
    }

    /// Remove the orphan events that should not wait for another flush and
    /// return them as failures to record: all of them when `carry` is false,
    /// otherwise the ones received more than [`ORPHAN_EVENT_GRACE_NANOS`] before
    /// `now_nanos`. Only those `may_record` accepts, by client IP, are returned.
    pub(super) fn take_expired_orphan_events(
        &mut self,
        now_nanos: i64,
        carry: bool,
        mut may_record: impl FnMut(Option<IpAddr>) -> bool,
    ) -> Vec<IngestRowFailure> {
        if self.orphan_events.is_empty() {
            return Vec::new();
        }
        let (expired, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.orphan_events)
            .into_iter()
            .partition(|orphan| {
                !carry
                    || now_nanos.saturating_sub(orphan.received_nanos) >= ORPHAN_EVENT_GRACE_NANOS
            });
        self.orphan_events = waiting;
        expired
            .into_iter()
            .filter(|orphan| may_record(orphan.client_ip))
            .map(|orphan| unknown_page_failure(orphan.wire_json))
            .collect()
    }

    /// Events ready to be written, for showing how much is waiting on a flush.
    pub(crate) fn queued_event_count(&self) -> usize {
        self.events.len()
    }

    /// Drop the queued rows and lookup maps after the rows have been written.
    /// Orphan events stay: they were not written, and their pageview may still
    /// arrive.
    pub(super) fn clear(&mut self) {
        self.pageviews.clear();
        self.landings.clear();
        self.events.clear();
        self.engagements.clear();
        self.metadata_updates.clear();
        self.context_updates.clear();
        self.forgets.clear();
        self.crawler_views.clear();
        self.pageview_idx_by_client_id.clear();
        self.pageview_idx_by_ts.clear();
        self.visitor_by_user_hash.clear();
        self.queued_pageview_ts.clear();
        self.queued_event_ts.clear();
        // The rows we just wrote set new database maxima, and their ids are no
        // longer tracked in memory, so the floors have to be read again.
        self.db_max_pageview_ts = None;
        self.db_max_event_ts = None;
        self.total_events = 0;
    }

    /// Fully reset for slot recycling: release memory and drop the id bookkeeping.
    pub(super) fn reset(&mut self) {
        self.pageviews = Vec::new();
        self.landings = Vec::new();
        self.events = Vec::new();
        self.engagements = Vec::new();
        self.metadata_updates = Vec::new();
        self.context_updates = Vec::new();
        self.forgets = Vec::new();
        self.orphan_events = Vec::new();
        self.crawler_views = FxHashMap::default();
        self.pageview_idx_by_client_id = FxHashMap::default();
        self.pageview_idx_by_ts = FxHashMap::default();
        self.visitor_by_user_hash = FxHashMap::default();
        self.queued_pageview_ts = FxHashSet::default();
        self.queued_event_ts = FxHashSet::default();
        self.db_max_pageview_ts = None;
        self.db_max_event_ts = None;
        self.total_events = 0;
    }

    pub(super) fn pageview_ids(&self, index: PageviewRef) -> (i64, i64) {
        let page = &self.pageviews[index as usize];
        (page.timestamp_id, page.visitor_timestamp_id)
    }

    /// JSON for a queued row, stored on the `errors` row that records why the
    /// flush could not write it.
    pub(super) fn row_json(&self, row: BatchRow) -> String {
        let dropped = match row {
            BatchRow::Pageview(index) => self.pageviews.get(index).map(DroppedRow::Pageview),
            BatchRow::Landing(index) => self.landings.get(index).map(DroppedRow::Landing),
            BatchRow::Event(index) => self.events.get(index).map(DroppedRow::Event),
            BatchRow::MetadataUpdate(index) => self
                .metadata_updates
                .get(index)
                .map(DroppedRow::MetadataUpdate),
            BatchRow::ContextUpdate(index) => self
                .context_updates
                .get(index)
                .map(DroppedRow::ContextUpdate),
            BatchRow::Forget(index) => self.forgets.get(index).map(DroppedRow::Forget),
            BatchRow::EngagementsForPage(page_timestamp_id) => Some(DroppedRow::Engagements {
                page_timestamp_id,
                engagements: self
                    .engagements
                    .iter()
                    .filter(|engagement| engagement.page_timestamp_id == page_timestamp_id)
                    .collect(),
            }),
        };
        match dropped {
            Some(dropped) => serde_json::to_string(&dropped).unwrap_or_else(|_| "null".to_string()),
            None => "null".to_string(),
        }
    }

    /// Drop a row the project database rejected so the rest of the batch can be
    /// flushed without it. `total_events` is left alone: it counts the hits we
    /// accepted and orders metadata writes, and the batch is cleared right after
    /// the flush anyway.
    pub(super) fn remove_row(&mut self, row: BatchRow) {
        match row {
            BatchRow::Pageview(index) => self.remove_pageview(index),
            BatchRow::Landing(index) => {
                remove_at(&mut self.landings, index);
            }
            BatchRow::Event(index) => {
                if let Some(event) = remove_at(&mut self.events, index) {
                    self.queued_event_ts.remove(&event.timestamp_id);
                }
            }
            BatchRow::MetadataUpdate(index) => {
                remove_at(&mut self.metadata_updates, index);
            }
            BatchRow::ContextUpdate(index) => {
                remove_at(&mut self.context_updates, index);
            }
            BatchRow::Forget(index) => {
                remove_at(&mut self.forgets, index);
            }
            BatchRow::EngagementsForPage(page_timestamp_id) => self
                .engagements
                .retain(|engagement| engagement.page_timestamp_id != page_timestamp_id),
        }
    }

    fn remove_pageview(&mut self, index: usize) {
        let Some(page) = remove_at(&mut self.pageviews, index) else {
            return;
        };
        self.queued_pageview_ts.remove(&page.timestamp_id);
        self.pageview_idx_by_ts.remove(&page.timestamp_id);
        self.pageview_idx_by_client_id.remove(&page.client_page_id);
        // The remaining pageviews shifted down by one.
        let removed = index as PageviewRef;
        for reference in self
            .pageview_idx_by_ts
            .values_mut()
            .chain(self.pageview_idx_by_client_id.values_mut())
        {
            if *reference > removed {
                *reference -= 1;
            }
        }
        // A landing row keys off its pageview's timestamp id, so it cannot be
        // written once that pageview is gone.
        self.landings
            .retain(|landing| landing.timestamp_id != page.timestamp_id);
    }
}

fn remove_at<T>(rows: &mut Vec<T>, index: usize) -> Option<T> {
    (index < rows.len()).then(|| rows.remove(index))
}

/// Identifies one queued row (or, for engagements, the rows aggregated into one
/// write) so a flush failure can be blamed on it, recorded, and dropped before
/// the flush is retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchRow {
    Pageview(usize),
    Landing(usize),
    Event(usize),
    MetadataUpdate(usize),
    ContextUpdate(usize),
    Forget(usize),
    /// Engagements are aggregated by their target pageview before they are
    /// written, so a failed write blames every engagement for that pageview.
    EngagementsForPage(i64),
}

/// The queued row a flush could not write, serialized into the `data` column of
/// its `errors` row. The `row` tag names which kind of row it was.
#[derive(Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
enum DroppedRow<'a> {
    Pageview(&'a PageView),
    Landing(&'a LandingPageView),
    Event(&'a EventRow),
    MetadataUpdate(&'a MetadataUpdateRow),
    ContextUpdate(&'a LandingContextRow),
    Forget(&'a ForgetRow),
    Engagements {
        page_timestamp_id: i64,
        engagements: Vec<&'a EngagementRow>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::map::IngestBatch;
    use crate::ingest::test_support::{
        sample_landing_pageview, sample_orphan_event, sample_pageview,
    };

    #[test]
    fn flush_carries_orphan_events_only_within_their_grace_window() {
        let now = 100 * ORPHAN_EVENT_GRACE_NANOS;
        let mut batch = IngestBatch::new(0, 0);
        batch
            .orphan_events
            .push(sample_orphan_event(now - ORPHAN_EVENT_GRACE_NANOS));
        batch.orphan_events.push(sample_orphan_event(now - 1));

        let dropped = batch.take_expired_orphan_events(now, true, |_| true);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].error_type, "unknown_page");
        let expected_data = format!(r#"{{"received":{}}}"#, now - ORPHAN_EVENT_GRACE_NANOS);
        assert_eq!(dropped[0].data, expected_data);

        // A flush that writes the rest of the batch keeps the carried event.
        batch.clear();
        assert_eq!(batch.orphan_events.len(), 1);
        assert!(!batch.is_empty());

        let dropped = batch.take_expired_orphan_events(now, false, |_| true);
        assert_eq!(dropped.len(), 1);
        assert!(batch.is_empty());
    }

    #[test]
    fn an_orphan_over_the_error_limit_is_dropped_without_a_record() {
        let mut batch = IngestBatch::new(0, 0);
        let over_limit: IpAddr = "192.0.2.1".parse().expect("ip");
        let mut unrecorded = sample_orphan_event(0);
        unrecorded.client_ip = Some(over_limit);
        batch.orphan_events.push(unrecorded);
        batch.orphan_events.push(sample_orphan_event(0));

        let dropped = batch.take_expired_orphan_events(0, false, |ip| ip != Some(over_limit));
        assert_eq!(dropped.len(), 1);
        assert!(batch.orphan_events.is_empty());
    }

    #[test]
    fn abandoned_batch_json_includes_queued_rows() {
        let mut batch = IngestBatch::new(7, 0);
        {
            let data: &mut BatchData = &mut batch;
            data.pageviews.push(sample_pageview(1_000, 1_000, 3));
            data.landings.push(sample_landing_pageview(1_000));
            data.total_events = 2;
        }
        let data: &BatchData = &batch;
        let json = serde_json::to_value(data).expect("serialize");
        assert_eq!(json["pageviews"][0]["timestamp_id"], 1_000);
        assert_eq!(json["landings"][0]["timestamp_id"], 1_000);
        assert!(json.get("pageview_idx_by_ts").is_none());
    }
}
