use dioxus::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

/// Sparse, non-contiguous list backed by a BTreeMap.
///
/// Only indices that have been loaded occupy memory. `total_count` is the logical
/// length of the list (e.g. IMAP EXISTS), which may be much larger than the number
/// of cached entries. Missing indices are the signal that data still needs fetching.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseList<T: Clone> {
    items: BTreeMap<usize, T>,
    total_count: usize,
}

impl<T: Clone> SparseList<T> {
    pub fn new(total_count: usize) -> Self {
        Self {
            items: BTreeMap::new(),
            total_count,
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.total_count = 0;
    }

    pub fn insert(&mut self, index: usize, item: T) {
        if index < self.total_count {
            self.items.insert(index, item);
        }
    }

    /// Insert a contiguous batch starting at `start_index`.
    /// Items beyond `total_count` are ignored.
    pub fn insert_batch(&mut self, start_index: usize, items: Vec<T>) {
        for (offset, item) in items.into_iter().enumerate() {
            self.insert(start_index + offset, item);
        }
    }

    pub fn prepend(&mut self, item: T) {
        let mut new_items = BTreeMap::new();
        for (key, value) in self.items.iter() {
            new_items.insert(key + 1, value.clone());
        }
        new_items.insert(0, item);
        self.items = new_items;
        self.total_count += 1;
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(&index)
    }

    /// Find a cached item by predicate (does not scan missing indices).
    pub fn find<F>(&self, mut pred: F) -> Option<&T>
    where
        F: FnMut(&T) -> bool,
    {
        self.items.values().find(|v| pred(v))
    }

    /// Index of the first cached item matching `pred`.
    pub fn position<F>(&self, mut pred: F) -> Option<usize>
    where
        F: FnMut(&T) -> bool,
    {
        self.items.iter().find(|(_, v)| pred(v)).map(|(k, _)| *k)
    }

    pub fn has_item(&self, index: usize) -> bool {
        self.items.contains_key(&index)
    }

    pub fn clear_range(&mut self, range: Range<usize>) {
        let keys_to_remove: Vec<usize> = self.items.range(range).map(|(k, _)| *k).collect();
        for key in keys_to_remove {
            self.items.remove(&key);
        }
    }

    pub fn total_count(&self) -> usize {
        self.total_count
    }

    pub fn set_total_count(&mut self, count: usize) {
        self.total_count = count;
        self.items.retain(|k, _| *k < count);
    }

    pub fn cached_count(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.values()
    }

    /// Cached rows in index order (holes are skipped).
    pub fn iter_indexed(&self) -> impl Iterator<Item = (usize, &T)> {
        self.items.iter().map(|(k, v)| (*k, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.values_mut()
    }

    /// Remove matching cached items and close the index gaps (newest-first list).
    /// Returns each removed row with its original index.
    pub fn take_matching<F>(&mut self, mut pred: F) -> Vec<(usize, T)>
    where
        F: FnMut(&T) -> bool,
    {
        let remove_keys: Vec<usize> = self
            .items
            .iter()
            .filter(|(_, v)| pred(v))
            .map(|(k, _)| *k)
            .collect();
        if remove_keys.is_empty() {
            return Vec::new();
        }
        let mut taken = Vec::with_capacity(remove_keys.len());
        let mut new_items = BTreeMap::new();
        let mut removed = 0usize;
        let mut remove_idx = 0usize;
        for (k, v) in std::mem::take(&mut self.items) {
            while remove_idx < remove_keys.len() && remove_keys[remove_idx] < k {
                remove_idx += 1;
                removed += 1;
            }
            if remove_idx < remove_keys.len() && remove_keys[remove_idx] == k {
                taken.push((k, v));
                remove_idx += 1;
                removed += 1;
                continue;
            }
            new_items.insert(k - removed, v);
        }
        self.items = new_items;
        self.total_count = self.total_count.saturating_sub(taken.len());
        taken
    }

    /// Remove matching cached items and close the index gaps (newest-first list).
    pub fn remove_matching<F>(&mut self, pred: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        self.take_matching(pred).len()
    }

    /// Move the logical row at `from` to `to` without changing `total_count`.
    ///
    /// Cached items (and holes) between the two indices shift to close the gap.
    /// A missing `from` still shifts neighbours so uncached rows stay aligned.
    pub fn relocate(&mut self, from: usize, to: usize) {
        if from == to || from >= self.total_count || self.total_count == 0 {
            return;
        }
        let to = to.min(self.total_count - 1);
        let item = self.items.remove(&from);
        if from < to {
            let keys: Vec<usize> = self
                .items
                .keys()
                .copied()
                .filter(|&k| k > from && k <= to)
                .collect();
            for k in keys {
                if let Some(v) = self.items.remove(&k) {
                    self.items.insert(k - 1, v);
                }
            }
        } else {
            let keys: Vec<usize> = self
                .items
                .keys()
                .copied()
                .filter(|&k| k >= to && k < from)
                .rev()
                .collect();
            for k in keys {
                if let Some(v) = self.items.remove(&k) {
                    self.items.insert(k + 1, v);
                }
            }
        }
        if let Some(item) = item {
            self.items.insert(to, item);
        }
    }

    /// Insert `item` at `index`, shifting later cached rows up.
    pub fn insert_at(&mut self, index: usize, item: T) {
        let index = index.min(self.total_count);
        let to_shift: Vec<usize> = self
            .items
            .keys()
            .copied()
            .filter(|&k| k >= index)
            .rev()
            .collect();
        for k in to_shift {
            if let Some(v) = self.items.remove(&k) {
                self.items.insert(k + 1, v);
            }
        }
        self.items.insert(index, item);
        self.total_count += 1;
    }

    /// Find contiguous runs of missing indices within `[start, end)`.
    pub fn missing_ranges(&self, start: usize, end: usize) -> Vec<Range<usize>> {
        let end = end.min(self.total_count);
        let start = start.min(end);
        let mut ranges = Vec::new();
        let mut gap_start: Option<usize> = None;

        for i in start..end {
            if self.has_item(i) {
                if let Some(gs) = gap_start.take() {
                    ranges.push(gs..i);
                }
            } else if gap_start.is_none() {
                gap_start = Some(i);
            }
        }
        if let Some(gs) = gap_start {
            ranges.push(gs..end);
        }
        ranges
    }

    /// Drop cached items far from the viewport when over `max_cached`.
    pub fn evict_outside(&mut self, keep: Range<usize>, max_cached: usize) {
        if self.items.len() <= max_cached {
            return;
        }
        let keep_start = keep.start;
        let keep_end = keep.end.min(self.total_count);
        self.items.retain(|&k, _| k >= keep_start && k < keep_end);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportInfo {
    pub scroll_top: f64,
    pub viewport_height: f64,
    pub first_visible_index: usize,
    pub last_visible_index: usize,
    pub visible_count: usize,
}

impl ViewportInfo {
    pub fn calculate(
        scroll_top: f64,
        viewport_height: f64,
        item_height: f64,
        total_items: usize,
    ) -> Self {
        if total_items == 0 || item_height <= 0.0 || viewport_height <= 0.0 {
            return Self {
                scroll_top,
                viewport_height: viewport_height.max(0.0),
                first_visible_index: 0,
                last_visible_index: 0,
                visible_count: 0,
            };
        }
        let first_visible = (scroll_top / item_height).floor() as usize;
        let first_visible = first_visible.min(total_items.saturating_sub(1));
        let visible_count = (viewport_height / item_height).ceil() as usize + 1;
        let last_visible = (first_visible + visible_count - 1).min(total_items.saturating_sub(1));

        Self {
            scroll_top,
            viewport_height,
            first_visible_index: first_visible,
            last_visible_index: last_visible,
            visible_count,
        }
    }

    pub fn buffered_range(&self, buffer_size: usize, total_items: usize) -> Range<usize> {
        if self.visible_count == 0 || total_items == 0 {
            return 0..0;
        }
        let start = self.first_visible_index.saturating_sub(buffer_size);
        let end = (self.last_visible_index + buffer_size + 1).min(total_items);
        start..end
    }
}

/// After removing the row at `removed_index`, the list index that should stay
/// selected. Prefers the row that slid into that slot (visually next / older);
/// if that was the last row, the new last row (previous / newer).
pub fn index_after_removal(total_after: usize, removed_index: usize) -> Option<usize> {
    if total_after == 0 {
        None
    } else if removed_index < total_after {
        Some(removed_index)
    } else {
        Some(total_after - 1)
    }
}

/// Next list index for a keyboard move. `None` means stay put.
///
/// Newest-first lists: `delta > 0` is visually down (older), `delta < 0` is up.
/// With no current row, both directions land on index 0.
pub fn adjacent_index(total: usize, current: Option<usize>, delta: i32) -> Option<usize> {
    if total == 0 {
        return None;
    }
    match current {
        None => Some(0),
        Some(i) => {
            let next = i as i64 + i64::from(delta);
            if next < 0 || next >= total as i64 {
                None
            } else {
                Some(next as usize)
            }
        }
    }
}

/// Result of walking the list for the next/previous unread row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadScan {
    /// Loaded unread row at this index.
    Found(usize),
    /// First unloaded index in the scan direction.
    Hole(usize),
    /// Reached the list edge with no unread row.
    None,
}

/// Walk from `current` by `delta` (`+1` next, `-1` previous) until an unread
/// row, a hole, or the list edge.
///
/// `is_unread(i)` is `Some(true)` unread, `Some(false)` read, `None` not loaded.
/// With no current row, next starts at 0 and previous at `total - 1`.
pub fn next_unread_index(
    total: usize,
    current: Option<usize>,
    delta: i32,
    mut is_unread: impl FnMut(usize) -> Option<bool>,
) -> UnreadScan {
    if total == 0 || delta == 0 {
        return UnreadScan::None;
    }
    let step: i64 = if delta > 0 { 1 } else { -1 };
    let mut i = match current {
        Some(idx) => idx as i64 + step,
        None if step > 0 => 0,
        None => total as i64 - 1,
    };
    let last = total as i64;
    while i >= 0 && i < last {
        let idx = i as usize;
        match is_unread(idx) {
            Some(true) => return UnreadScan::Found(idx),
            Some(false) => i += step,
            None => return UnreadScan::Hole(idx),
        }
    }
    UnreadScan::None
}

/// Exclusive start index for an unread scan (`next_unread_index`).
///
/// `stored` is the focus index from when the row was selected; `live` is where
/// that message is now. Unread-first auto-mark relocates the row into the read
/// section and slides the next unread into `stored`, so a forward scan must
/// include that slot (`stored - 1`). Backward scans still use `stored`.
pub fn unread_scan_from(stored: Option<usize>, live: Option<usize>, delta: i32) -> Option<usize> {
    match (stored, live) {
        (Some(stored), Some(live)) if live != stored && delta > 0 => stored.checked_sub(1),
        (Some(stored), _) => Some(stored),
        (_, live) => live,
    }
}

/// Exclusive start so the next scan re-checks `hole` (now loaded) without
/// walking the already-examined prefix again.
pub fn unread_scan_resume(hole: usize, delta: i32) -> Option<usize> {
    if delta > 0 {
        hole.checked_sub(1)
    } else {
        Some(hole.saturating_add(1))
    }
}

/// Subtract already-pending ranges so we do not re-request in-flight data.
fn subtract_pending(needed: Vec<Range<usize>>, pending: &[Range<usize>]) -> Vec<Range<usize>> {
    if pending.is_empty() {
        return needed;
    }
    let mut result = Vec::new();
    for range in needed {
        let mut segments = vec![range];
        for p in pending {
            let mut next = Vec::new();
            for seg in segments {
                if seg.end <= p.start || seg.start >= p.end {
                    next.push(seg);
                } else {
                    if seg.start < p.start {
                        next.push(seg.start..p.start);
                    }
                    if seg.end > p.end {
                        next.push(p.end..seg.end);
                    }
                }
            }
            segments = next;
        }
        result.extend(segments);
    }
    result
}

fn ranges_fully_loaded<T: Clone>(items: &SparseList<T>, range: &Range<usize>) -> bool {
    (range.start..range.end).all(|i| items.has_item(i))
}

/// Queue fetches for missing ranges. `pending` is non-reactive bookkeeping.
fn queue_missing_fetches<T: Clone>(
    items: &SparseList<T>,
    buffered: Range<usize>,
    pending: &RefCell<Vec<Range<usize>>>,
    on_need_range: EventHandler<Range<usize>>,
) {
    if items.total_count() == 0 || buffered.start >= buffered.end {
        return;
    }

    pending
        .borrow_mut()
        .retain(|r| !ranges_fully_loaded(items, r));

    let to_request = {
        let needed = items.missing_ranges(buffered.start, buffered.end);
        let pending_guard = pending.borrow();
        subtract_pending(needed, &pending_guard)
    };

    if to_request.is_empty() {
        return;
    }

    pending.borrow_mut().extend(to_request.iter().cloned());
    for range in to_request {
        on_need_range.call(range);
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct VirtualScrollProps<T>
where
    T: Clone + PartialEq + 'static,
{
    /// Sparse cache of loaded items (shared with the data layer).
    pub items: Signal<SparseList<T>>,
    pub item_height: f64,
    /// Extra rows above/below the viewport to pre-fetch and pre-render.
    pub buffer_size: usize,
    /// Fired when a contiguous range of missing indices should be loaded.
    pub on_need_range: EventHandler<Range<usize>>,
    pub render_item: Callback<(usize, T), Element>,
    #[props(!optional)]
    pub debounce_ms: Option<u32>,
    /// Soft cap on cached items; entries outside the buffered viewport are dropped.
    #[props(!optional)]
    pub max_cached: Option<usize>,
    /// Scroll so this row is visible when it changes (keyboard selection).
    #[props(!optional)]
    pub reveal_index: Option<usize>,
}

#[component]
pub fn VirtualScroll<T>(props: VirtualScrollProps<T>) -> Element
where
    T: Clone + PartialEq + 'static,
{
    // Measured content-box height from ResizeObserver; 0 until first observation.
    let mut measured_height = use_signal(|| 0.0f64);
    let mut viewport_info = use_signal(|| ViewportInfo::calculate(0.0, 0.0, props.item_height, 0));
    // Non-reactive bookkeeping — must not be a Signal (writing Signals inside
    // effects that also read them re-triggers the effect forever and freezes wasm).
    let pending_ranges = use_hook(|| Rc::new(RefCell::new(Vec::<Range<usize>>::new())));
    let last_total = use_hook(|| Rc::new(Cell::new(0usize)));
    let current_item_height = use_hook(|| Rc::new(Cell::new(props.item_height)));
    let mut scroll_generation = use_signal(|| 0u64);
    let mut container_ref = use_signal(|| None::<Rc<MountedData>>);

    // React to `items`, measured height, and row height. Never write reactive
    // state that this effect also reads with `.read()` (use peek for viewport).
    let item_height = props.item_height;
    let items_signal = props.items;
    let buffer_size = props.buffer_size;
    let on_need_range = props.on_need_range;
    let pending_for_viewport = pending_ranges.clone();
    let last_total_for_viewport = last_total.clone();
    let height_for_viewport = current_item_height.clone();
    use_effect(use_reactive!(|item_height| {
        if height_for_viewport.get() != item_height {
            height_for_viewport.set(item_height);
            // Drop in-flight scroll tasks that captured the previous row height.
            let next = *scroll_generation.peek() + 1;
            scroll_generation.set(next);
        }

        let items = items_signal.read().clone();
        let total = items.total_count();
        let height = *measured_height.read();

        let total_changed = total != last_total_for_viewport.get();
        if total_changed {
            last_total_for_viewport.set(total);
            pending_for_viewport.borrow_mut().clear();
        }

        let scroll_top = if total_changed {
            0.0
        } else {
            viewport_info.peek().scroll_top
        };
        let new_vp = ViewportInfo::calculate(scroll_top, height, item_height, total);
        if new_vp != *viewport_info.peek() {
            viewport_info.set(new_vp);
        }

        // Wait for ResizeObserver before requesting data so we only fetch
        // what fits the real viewport (+ buffer).
        if height <= 0.0 {
            return;
        }

        let buffered = new_vp.buffered_range(buffer_size, total);
        queue_missing_fetches(&items, buffered, &pending_for_viewport, on_need_range);
    }));

    let reveal_index = props.reveal_index;
    use_effect(use_reactive!(|reveal_index| {
        if let Some(index) = reveal_index {
            scroll_row_into_view(index, item_height);
        }
    }));

    let props_for_scroll = props.clone();
    let pending_for_scroll = pending_ranges.clone();
    let height_for_scroll = current_item_height.clone();
    let container_clone = container_ref;
    let handle_scroll = move |_| {
        let props = props_for_scroll.clone();
        let pending_ranges = pending_for_scroll.clone();
        let current_item_height = height_for_scroll.clone();
        let generation = {
            let next = *scroll_generation.peek() + 1;
            scroll_generation.set(next);
            next
        };
        spawn(async move {
            if let Some(element) = container_clone.read().as_ref() {
                if let Ok(offset) = element.get_scroll_offset().await {
                    if *scroll_generation.peek() != generation {
                        return;
                    }
                    let total = props.items.peek().total_count();
                    let height = *measured_height.peek();
                    let vp =
                        ViewportInfo::calculate(offset.y, height, current_item_height.get(), total);
                    viewport_info.set(vp);
                }
            }

            if let Some(debounce_ms) = props.debounce_ms {
                sleep_ms(debounce_ms).await;
            }
            if *scroll_generation.peek() != generation {
                return;
            }

            let items = props.items.peek().clone();
            let total = items.total_count();
            let height = *measured_height.peek();
            if height <= 0.0 {
                return;
            }
            let vp = *viewport_info.peek();
            let buffered = vp.buffered_range(props.buffer_size, total);
            queue_missing_fetches(&items, buffered, &pending_ranges, props.on_need_range);

            if let Some(max_cached) = props.max_cached {
                if items.cached_count() > max_cached {
                    let keep = vp.buffered_range(props.buffer_size * 2, total);
                    let mut items_signal = props.items;
                    items_signal.write().evict_outside(keep, max_cached);
                }
            }
        });
    };

    let handle_resize = move |evt: Event<ResizeData>| {
        let height = evt
            .data()
            .get_content_box_size()
            .map(|s| s.height)
            .or_else(|_| evt.data().get_border_box_size().map(|s| s.height))
            .unwrap_or(0.0);

        let prev = *measured_height.peek();
        // Ignore sub-pixel noise from the observer.
        if (height - prev).abs() < 0.5 {
            return;
        }
        measured_height.set(height.max(0.0));
        // The items/height effect recalculates the viewport and queues fetches.
    };

    let total = props.items.read().total_count();
    let total_height = total as f64 * props.item_height;
    let vp = *viewport_info.read();
    let render_start = if vp.visible_count == 0 {
        0
    } else {
        vp.first_visible_index.saturating_sub(props.buffer_size)
    };
    let render_end = if vp.visible_count == 0 {
        0
    } else {
        (vp.last_visible_index + props.buffer_size + 1).min(total)
    };

    let items_to_render: Vec<(usize, Option<T>)> = {
        let items = props.items.read();
        (render_start..render_end)
            .map(|index| (index, items.get(index).cloned()))
            .collect()
    };

    rsx! {
        div {
            id: "message-list-scroll",
            class: "virtual-scroll-container",
            // Fill the parent; height is measured via ResizeObserver (onresize).
            style: "position: relative; height: 100%; width: 100%; overflow-y: auto; scrollbar-gutter: stable;",
            onscroll: handle_scroll,
            onresize: handle_resize,
            onmounted: move |node_ref| {
                let data = node_ref.data();
                container_ref.set(Some(data.clone()));
                // Bootstrap size if ResizeObserver is delayed; onresize will refine it.
                spawn(async move {
                    if let Ok(rect) = data.get_client_rect().await {
                        let height = rect.height();
                        if height > 0.0 && (*measured_height.peek() - height).abs() >= 0.5 {
                            measured_height.set(height);
                        }
                    }
                });
            },

            div {
                class: "virtual-scroll-spacer",
                style: "height: {total_height}px; position: relative;",

                div {
                    class: "virtual-scroll-content",
                    style: "transform: translateY({render_start as f64 * props.item_height}px); position: absolute; top: 0; left: 0; right: 0;",

                    for (index, item) in items_to_render {
                        if let Some(item) = item {
                            div {
                                key: "{index}",
                                class: "virtual-scroll-item",
                                style: "height: {props.item_height}px;",
                                {(props.render_item)((index, item))}
                            }
                        } else {
                            div {
                                key: "{index}",
                                class: "virtual-scroll-placeholder",
                                style: "height: {props.item_height}px; display: flex; align-items: center; padding: 8px;",
                                div {
                                    class: "loading-skeleton",
                                    style: "height: 60%; width: 100%; background: linear-gradient(90deg, #f0f0f0 25%, #f8f8f8 50%, #f0f0f0 75%); background-size: 200% 100%; animation: shimmer 1.5s infinite; border-radius: 4px;",
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn scroll_row_into_view(index: usize, item_height: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(el) = doc.get_element_by_id("message-list-scroll") else {
            return;
        };
        let top = index as f64 * item_height;
        let view_top = f64::from(el.scroll_top());
        let height = f64::from(el.client_height());
        if height <= 0.0 {
            return;
        }
        if top < view_top {
            el.set_scroll_top(top.round() as i32);
        } else if top + item_height > view_top + height {
            el.set_scroll_top((top + item_height - height).round() as i32);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (index, item_height);
    }
}

/// Sleep that works on both wasm (no `std::time::Instant` / tokio time) and native.
async fn sleep_ms(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_list_missing_ranges() {
        let mut list = SparseList::new(100);
        list.insert(0, "a");
        list.insert(1, "b");
        list.insert(5, "c");
        let gaps = list.missing_ranges(0, 10);
        assert_eq!(gaps, vec![2..5, 6..10]);
    }

    #[test]
    fn cached_prefix_still_reports_holes_for_incremental_fetch() {
        // Mail cache stores only a contiguous head; virtual scroll must keep
        // requesting the unloaded tail (and any interior holes).
        let mut list = SparseList::new(40);
        list.insert_batch(0, vec!["m0", "m1", "m2", "m3", "m4"]);
        assert!(list.missing_ranges(0, 5).is_empty());
        assert_eq!(list.missing_ranges(0, 12), vec![5..12]);
        assert_eq!(list.missing_ranges(20, 25), vec![20..25]);
    }

    #[test]
    fn sparse_list_no_dense_allocation() {
        let mut list = SparseList::new(60_000);
        list.insert(0, 1);
        list.insert(59_999, 2);
        assert_eq!(list.cached_count(), 2);
        assert_eq!(list.total_count(), 60_000);
        assert!(!list.has_item(1));
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(
            list.iter_indexed()
                .map(|(i, v)| (i, *v))
                .collect::<Vec<_>>(),
            vec![(0, 1), (59_999, 2)]
        );
    }

    #[test]
    fn subtract_pending_splits_ranges() {
        let needed = vec![0..20];
        let pending = vec![5..10];
        let result = subtract_pending(needed, &pending);
        assert_eq!(result, vec![0..5, 10..20]);
    }

    #[test]
    fn viewport_zero_height_requests_nothing() {
        let vp = ViewportInfo::calculate(0.0, 0.0, 72.0, 1000);
        assert_eq!(vp.visible_count, 0);
        assert_eq!(vp.buffered_range(15, 1000), 0..0);
    }

    #[test]
    fn sparse_list_remove_matching_shifts_indices() {
        let mut list = SparseList::new(5);
        list.insert(0, "a");
        list.insert(1, "b");
        list.insert(3, "d");
        list.insert(4, "e");
        let n = list.remove_matching(|s| *s == "b");
        assert_eq!(n, 1);
        assert_eq!(list.total_count(), 4);
        assert_eq!(list.get(0).copied(), Some("a"));
        assert!(!list.has_item(1));
        assert_eq!(list.get(2).copied(), Some("d"));
        assert_eq!(list.get(3).copied(), Some("e"));
    }

    #[test]
    fn sparse_list_take_and_insert_at_restores() {
        let mut list = SparseList::new(4);
        list.insert(0, "a");
        list.insert(1, "b");
        list.insert(2, "c");
        list.insert(3, "d");
        let taken = list.take_matching(|s| *s == "b" || *s == "d");
        assert_eq!(taken.len(), 2);
        assert_eq!(list.total_count(), 2);
        list.insert_at(taken[0].0, taken[0].1);
        list.insert_at(taken[1].0, taken[1].1);
        assert_eq!(list.total_count(), 4);
        assert_eq!(list.get(0).copied(), Some("a"));
        assert_eq!(list.get(1).copied(), Some("b"));
        assert_eq!(list.get(2).copied(), Some("c"));
        assert_eq!(list.get(3).copied(), Some("d"));
    }

    #[test]
    fn viewport_measured_height_drives_visible_count() {
        // 320px / 72px ≈ 4.44 → ceil + 1 = 6 visible rows
        let vp = ViewportInfo::calculate(0.0, 320.0, 72.0, 1000);
        assert_eq!(vp.first_visible_index, 0);
        assert_eq!(vp.visible_count, 6);
        assert_eq!(vp.last_visible_index, 5);
        assert_eq!(vp.buffered_range(2, 1000), 0..8);
    }

    #[test]
    fn viewport_recalculates_when_item_height_changes() {
        let comfortable = ViewportInfo::calculate(520.0, 320.0, 52.0, 1000);
        let compact = ViewportInfo::calculate(520.0, 320.0, 40.0, 1000);
        assert_eq!(comfortable.first_visible_index, 10);
        assert_eq!(compact.first_visible_index, 13);
        assert!(compact.visible_count > comfortable.visible_count);
    }

    #[test]
    fn sparse_list_relocate_down_shifts_between() {
        let mut list = SparseList::new(5);
        list.insert(0, "a");
        list.insert(1, "b");
        list.insert(2, "c");
        list.insert(4, "e");
        list.relocate(1, 3);
        assert_eq!(list.total_count(), 5);
        assert_eq!(list.get(0).copied(), Some("a"));
        assert_eq!(list.get(1).copied(), Some("c"));
        assert!(!list.has_item(2));
        assert_eq!(list.get(3).copied(), Some("b"));
        assert_eq!(list.get(4).copied(), Some("e"));
    }

    #[test]
    fn sparse_list_relocate_up_shifts_between() {
        let mut list = SparseList::new(4);
        list.insert(0, "a");
        list.insert(1, "b");
        list.insert(2, "c");
        list.insert(3, "d");
        list.relocate(3, 1);
        assert_eq!(
            (0..4)
                .filter_map(|i| list.get(i).copied())
                .collect::<Vec<_>>(),
            ["a", "d", "b", "c"]
        );
    }

    #[test]
    fn sparse_list_position_finds_cached_index() {
        let mut list = SparseList::new(10);
        list.insert(0, "a");
        list.insert(4, "e");
        assert_eq!(list.position(|s| *s == "e"), Some(4));
        assert_eq!(list.position(|s| *s == "missing"), None);
    }

    #[test]
    fn index_after_removal_prefers_next_then_previous() {
        assert_eq!(index_after_removal(0, 0), None);
        assert_eq!(index_after_removal(2, 0), Some(0));
        assert_eq!(index_after_removal(2, 1), Some(1));
        assert_eq!(index_after_removal(2, 2), Some(1));
    }

    #[test]
    fn adjacent_index_moves_and_clamps() {
        assert_eq!(adjacent_index(0, None, 1), None);
        assert_eq!(adjacent_index(5, None, 1), Some(0));
        assert_eq!(adjacent_index(5, None, -1), Some(0));
        assert_eq!(adjacent_index(5, Some(0), 1), Some(1));
        assert_eq!(adjacent_index(5, Some(0), -1), None);
        assert_eq!(adjacent_index(5, Some(4), 1), None);
        assert_eq!(adjacent_index(5, Some(4), -1), Some(3));
    }

    /// `Some(true)` unread, `Some(false)` read, `None` hole.
    fn scan(rows: &[Option<bool>], current: Option<usize>, delta: i32) -> UnreadScan {
        next_unread_index(rows.len(), current, delta, |i| rows[i])
    }

    #[test]
    fn next_unread_index_finds_first_and_last_when_unfocused() {
        let rows = [Some(false), Some(true), Some(false), Some(true)];
        assert_eq!(scan(&rows, None, 1), UnreadScan::Found(1));
        assert_eq!(scan(&rows, None, -1), UnreadScan::Found(3));
        assert_eq!(scan(&[], None, 1), UnreadScan::None);
    }

    #[test]
    fn next_unread_index_skips_current_even_if_unread() {
        let rows = [Some(true), Some(false), Some(true)];
        assert_eq!(scan(&rows, Some(0), 1), UnreadScan::Found(2));
        assert_eq!(scan(&rows, Some(2), -1), UnreadScan::Found(0));
        assert_eq!(scan(&rows, Some(2), 1), UnreadScan::None);
        assert_eq!(scan(&rows, Some(0), -1), UnreadScan::None);
    }

    #[test]
    fn next_unread_index_stops_at_first_hole() {
        let rows = [Some(false), None, Some(true)];
        assert_eq!(scan(&rows, Some(0), 1), UnreadScan::Hole(1));
        let rows = [Some(true), None, Some(false)];
        assert_eq!(scan(&rows, Some(2), -1), UnreadScan::Hole(1));
    }

    #[test]
    fn next_unread_index_skips_read_rows() {
        let rows = [Some(true), Some(false), Some(false), Some(true)];
        assert_eq!(scan(&rows, Some(0), 1), UnreadScan::Found(3));
        assert_eq!(
            scan(&[Some(false), Some(false)], Some(0), 1),
            UnreadScan::None
        );
    }

    #[test]
    fn unread_scan_from_includes_vacated_slot_after_relocate() {
        // Selected unread at 2, then relocated to 5; next unread slid into 2.
        assert_eq!(unread_scan_from(Some(2), Some(5), 1), Some(1));
        let after_relocate = [Some(true), Some(true), Some(true), Some(false), Some(false)];
        assert_eq!(
            scan(&after_relocate, unread_scan_from(Some(2), Some(3), 1), 1),
            UnreadScan::Found(2)
        );
        assert_eq!(unread_scan_from(Some(0), Some(4), 1), None);
        assert_eq!(unread_scan_from(Some(2), Some(5), -1), Some(2));
        assert_eq!(unread_scan_from(Some(2), Some(2), 1), Some(2));
        assert_eq!(unread_scan_from(None, Some(3), 1), Some(3));
        assert_eq!(unread_scan_from(None, None, 1), None);
    }

    #[test]
    fn unread_scan_resume_rechecks_the_filled_hole() {
        assert_eq!(unread_scan_resume(0, 1), None);
        assert_eq!(unread_scan_resume(4, 1), Some(3));
        assert_eq!(unread_scan_resume(4, -1), Some(5));
    }
}
