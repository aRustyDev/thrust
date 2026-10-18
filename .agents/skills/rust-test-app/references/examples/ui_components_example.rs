// Template: the ui crate's src/lib.rs (components and their SSR tests). Example names: Entry, EntryList, Counter; rename to yours. [re-run]
//! Shared Dioxus components (rendered by the web, desktop and mobile shells).

use dioxus::prelude::*;

#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub id: u32,
    pub title: String,
    pub done: bool,
}

/// Pure view logic lives outside the component so it can be unit/property tested directly.
pub fn open_label(entries: &[Entry]) -> String {
    match entries.iter().filter(|e| !e.done).count() {
        1 => "1 item left".to_string(),
        n => format!("{n} items left"),
    }
}

#[component]
pub fn EntryList(entries: Vec<Entry>) -> Element {
    let label = open_label(&entries);
    rsx! {
        ul { class: "entries",
            for entry in entries {
                li {
                    key: "{entry.id}",
                    "data-testid": "entry-{entry.id}",
                    class: if entry.done { "done" } else { "open" },
                    "{entry.title}"
                }
            }
        }
        p { "data-testid": "remaining", "{label}" }
    }
}

#[component]
pub fn Counter(start: i32) -> Element {
    let mut count = use_signal(|| start);
    // Effects/async work run when the VirtualDom processes pending work.
    use_effect(move || {
        if count() < 0 {
            count.set(0);
        }
    });
    rsx! { span { "data-testid": "count", "{count}" } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use pretty_assertions::assert_eq;

    fn entry(id: u32, title: &str, done: bool) -> Entry {
        Entry { id, title: title.into(), done }
    }

    /// Mount a component with props, flush pending work (effects, spawned tasks), render to HTML.
    fn render_with<P: Clone + 'static>(component: fn(P) -> Element, props: P) -> String {
        let mut dom = VirtualDom::new_with_props(component, props);
        dom.rebuild_in_place();
        while dom.wait_for_work().now_or_never().is_some() {
            dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        }
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn entry_list_renders_items_and_remaining_count() {
        let html = render_with(
            EntryList,
            EntryListProps { entries: vec![entry(1, "alpha", false), entry(2, "beta", true)] },
        );
        // Assert the testid and class together, per item: `contains(class="done")` alone would still
        // pass if the class branch were inverted (entry-1 would take "done").
        assert!(html.contains(r#"data-testid="entry-1" class="open""#), "{html}");
        assert!(html.contains(r#"data-testid="entry-2" class="done""#), "{html}");
        assert!(html.contains("1 item left"), "{html}");
    }

    #[test]
    fn entry_list_matches_reference_markup() {
        let actual = render_with(EntryList, EntryListProps { entries: vec![] });
        let expected = dioxus_ssr::render_element(rsx! {
            ul { class: "entries" }
            p { "data-testid": "remaining", "0 items left" }
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn counter_effect_clamps_negative_start() {
        let html = render_with(Counter, CounterProps { start: -5 });
        assert!(html.contains(">0<"), "{html}");
    }

    #[test]
    fn open_label_pluralizes() {
        assert_eq!(open_label(&[]), "0 items left");
        assert_eq!(open_label(&[entry(1, "a", false)]), "1 item left");
    }
}
