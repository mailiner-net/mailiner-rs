use dioxus::prelude::*;
use dioxus::dioxus_core::SpawnIfAsync;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Clone)]
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

    pub fn insert(&mut self, index: usize, item: T) {
        if index < self.total_count {
            self.items.insert(index, item);
        }
    }

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

    pub fn has_item(&self, index: usize) -> bool {
        self.items.contains_key(&index)
    }

    pub fn clear_range(&mut self, range: Range<usize>) {
        let keys_to_remove: Vec<usize> = self.items
            .range(range)
            .map(|(k, _)| *k)
            .collect();

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
    pub fn calculate(scroll_top: f64, viewport_height: f64, item_height: f64, total_items: usize) -> Self {
        let first_visible = (scroll_top / item_height).floor() as usize;
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
}

#[derive(Props, Clone, PartialEq)]
pub struct VirtualScrollProps<T>
where
    T: Clone + PartialEq + 'static,
{
    pub total_items: usize,
    pub item_height: f64,
    pub viewport_height: f64,
    pub buffer_size: usize,
    pub fetch_threshold: usize,
    pub on_fetch: Callback<Range<usize>, Vec<T>>,
    pub render_item: Callback<(usize, &'static T), Element>,
    #[props(!optional)]
    pub debounce_ms: Option<u32>,
    #[props(!optional)]
    pub max_cached: Option<usize>,
}

pub struct VirtualScrollState<T: Clone> {
    pub items: SparseList<T>,
    pub viewport_info: ViewportInfo,
    pub is_fetching: Signal<bool>,
    pub pending_ranges: Signal<Vec<Range<usize>>>,
}

impl<T: Clone> VirtualScrollState<T> {
    pub fn new(total_items: usize) -> Self {
        Self {
            items: SparseList::new(total_items),
            viewport_info: ViewportInfo {
                scroll_top: 0.0,
                viewport_height: 0.0,
                first_visible_index: 0,
                last_visible_index: 0,
                visible_count: 0,
            },
            is_fetching: Signal::new(false),
            pending_ranges: Signal::new(Vec::new()),
        }
    }

    pub fn get_fetch_ranges(&self, buffer_size: usize, fetch_threshold: usize) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();

        let start = self.viewport_info.first_visible_index.saturating_sub(buffer_size);
        let end = (self.viewport_info.last_visible_index + buffer_size + 1)
            .min(self.items.total_count());

        let mut fetch_start = None;
        let mut consecutive_missing = 0;

        for i in start..end {
            if !self.items.has_item(i) {
                if fetch_start.is_none() {
                    fetch_start = Some(i);
                }
                consecutive_missing += 1;
            } else {
                if let Some(range_start) = fetch_start {
                    if consecutive_missing >= fetch_threshold {
                        ranges.push(range_start..i);
                    }
                }
                fetch_start = None;
                consecutive_missing = 0;
            }
        }

        if let Some(range_start) = fetch_start {
            if consecutive_missing >= fetch_threshold {
                ranges.push(range_start..end);
            }
        }

        ranges
    }

    pub fn clear_outside_viewport(&mut self, buffer_size: usize, max_cached: usize) {
        if self.items.cached_count() <= max_cached {
            return;
        }

        let keep_start = self.viewport_info.first_visible_index.saturating_sub(buffer_size * 2);
        let keep_end = (self.viewport_info.last_visible_index + buffer_size * 2)
            .min(self.items.total_count());

        if keep_start > 0 {
            self.items.clear_range(0..keep_start);
        }
        if keep_end < self.items.total_count() {
            self.items.clear_range(keep_end..self.items.total_count());
        }
    }
}

#[component]
pub fn VirtualScroll<T, F, R>(props: VirtualScrollProps<T>) -> Element
where
    T: Clone + PartialEq + 'static,
{
    let mut state = use_signal(|| VirtualScrollState::<T>::new(props.total_items));
    let mut scroll_timer = use_signal(|| None);
    let mut container_ref = use_signal(|| None::<Rc<MountedData>>);

    use_effect(move || {
        state.write().items.set_total_count(props.total_items);
    });

    let props_clone = props.clone();
    let container_clone = container_ref.clone();
    let handle_scroll = move |_| {
        let props_clone = props_clone.clone();
        spawn(async move {
            if let Some(element) = container_clone.read().as_ref() {
                let scroll_top = element.get_scroll_offset().await.unwrap().y;
                let viewport = ViewportInfo::calculate(
                    scroll_top,
                    props_clone.viewport_height,
                    props_clone.item_height,
                    props_clone.total_items,
                );

                state.write().viewport_info = viewport;
            }

            if let Some(debounce_ms) = props_clone.debounce_ms {
                scroll_timer.set(Some(std::time::Instant::now()));

                let state = state.clone();
                tokio::time::sleep(tokio::time::Duration::from_millis(debounce_ms as u64)).await;

                if let Some(timer_start) = scroll_timer.read().as_ref() {
                    if timer_start.elapsed().as_millis() >= debounce_ms as u128 {
                        trigger_fetch(state, props_clone).await;
                    }
                }
            } else {
                let state = state.clone();
                trigger_fetch(state, props_clone).await;
            }
        });
    };

    use_effect({
        let mut state = state.clone();
        let props = props.clone();
        move || {
            let props_clone = props.clone();
            spawn(async move {
                state.write().viewport_info = ViewportInfo::calculate(
                    0.0,
                    props_clone.viewport_height,
                    props_clone.item_height,
                    props_clone.total_items,
                );
                trigger_fetch(state, props_clone).await;
            });
        }
    });

    let total_height = props.total_items as f64 * props.item_height;
    let state_read = state.read();
    let viewport = state_read.viewport_info;

    let render_start = viewport.first_visible_index.saturating_sub(props.buffer_size);
    let render_end = (viewport.last_visible_index + props.buffer_size + 1)
        .min(props.total_items);

    rsx! {
        div {
            class: "virtual-scroll-container",
            style: "position: relative; height: {props.viewport_height}px; overflow-y: auto;",
            onscroll: handle_scroll,
            onmounted: move |node_ref| {
                container_ref.set(Some(node_ref.data));
            },

            div {
                class: "virtual-scroll-spacer",
                style: "height: {total_height}px; position: relative;",

                div {
                    class: "virtual-scroll-content",
                    style: "transform: translateY({render_start as f64 * props.item_height}px); position: absolute; top: 0; left: 0; right: 0;",

                    for index in render_start..render_end {
                        if let Some(item) = state().get_item(index) {
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

async fn trigger_fetch<T>(mut state: Signal<VirtualScrollState<T>>, props: VirtualScrollProps<T>)
where
    T: Clone + PartialEq + 'static,
{
    if *state.read().is_fetching.read() {
        return;
    }

    let ranges = state.read().get_fetch_ranges(props.buffer_size, props.fetch_threshold);

    if ranges.is_empty() {
        return;
    }

    state.write().is_fetching.set(true);
    state.write().pending_ranges.set(ranges.clone());

    for range in ranges {
        let items = (props.on_fetch)(range.clone()).spawn();
        state.write().items.insert_batch(range.start, items);
    }

    state.write().is_fetching.set(false);
    state.write().pending_ranges.set(Vec::new());

    if let Some(max_cached) = props.max_cached {
        state.write().clear_outside_viewport(props.buffer_size, max_cached);
    }
}

pub fn prepend_message<T: Clone>(state: &mut Signal<VirtualScrollState<T>>, message: T) {
    state.write().items.prepend(message);
}