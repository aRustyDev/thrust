# Dioxus component tests (VirtualDom + dioxus-ssr)

Dioxus **0.7.x** only. `[re-run]` on the archive's original `ui` crate: its four component tests pass
under nextest, namely the `render_with` flush loop, the reference-markup comparison, the effect clamp and
the plain helper function. Environment and logs: `SOURCES.md`. `examples/ui_components_example.rs` is
that file with identifiers renamed (`Todo` to `Entry`, `TodoList` to `EntryList`, `remaining_label` to
`open_label`, the test ids and strings) and one assertion strengthened (see "Control"). The shipped
template compiled and passed, and its class-branch control fails as described `[re-run]`
(`logs/verify-11.log`). Dioxus `main` is on 0.8 alphas (0.8.0-alpha.3 on 2026-10-16, per `dioxus-07`'s
`references/SOURCES.md`): do not copy alpha snippets, and re-check the crate setup and the
`NoOpMutations` path below when moving to 0.8 `[derived]`. Example names (`Entry`, `EntryList`) are
placeholders.

## Crate setup

Keep components in a platform-agnostic `ui` crate. Platform features (`web`, `desktop`, `mobile`,
`native`) belong to the launch crate, so component tests compile for the host with no wasm toolchain,
webview or system libraries. The crate built and its tests ran on the host with exactly these features
`[re-run]`; that none of those toolchains or libraries is needed is the source's claim `[zip]`:

```toml
[dependencies]
dioxus = { version = "0.7", default-features = false, features = ["macro", "html", "signals", "hooks"] }

[dev-dependencies]
dioxus-ssr = "0.7"
futures = "0.3"
pretty_assertions = "1"
```

**The four features suffice for components that use neither `asset!` nor `document::`** (the archive's
`ui` crate uses neither `[re-run]`). A crate whose components call `asset!` or `document::` will not
compile with only those four: add the `asset` and `document` features, or use `features = ["lib"]`. In
dioxus 0.7.10's `Cargo.toml` the `lib` feature is the four plus `mounted`, `document`, `asset`, `warnings`
and `cli-config` `[derived]` (read from the manifest, not built). This agrees with `dioxus-07`:
`references/legacy.md` names `features = ["lib"]` as the replacement for the old `dioxus-lib` crate, and
`references/cheatsheet.md` lists `lib` among the default features; `lib` is simply the larger set.

## Render with props, then flush

```rust
use futures::FutureExt;

/// Mount with props, flush pending work (effects, ready tasks), render to HTML.
fn render_with<P: Clone + 'static>(component: fn(P) -> Element, props: P) -> String {
    let mut dom = VirtualDom::new_with_props(component, props);
    dom.rebuild_in_place();
    while dom.wait_for_work().now_or_never().is_some() {
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
    }
    dioxus_ssr::render(&dom)
}
```

- `rebuild_in_place` does the first render and discards the mutations. Effects have **not** run yet
  `[derived]`.
- `wait_for_work().now_or_never()` takes only work that is already ready, then `render_immediate`
  applies it. The loop ends when nothing is ready, so it never waits on a timer or the network: this is
  why tests never sleep, and why fakes must be ready futures `[derived]`.
- Symptom of a missing flush loop: the HTML shows the initial value (`-5`) when an effect should have
  changed it to `0` `[derived]`.
- The loop is unbounded: an effect that triggers itself spins forever. In your own copy, count the
  iterations and panic past a cap (say 100) so such an effect fails the test instead of hanging it; the
  template does not do this `[derived]`.
- `#[component] fn EntryList(entries: Vec<Entry>)` generates `EntryListProps { entries }`.

```rust
#[test]
fn entry_list_renders_items_and_remaining_count() {
    let html = render_with(
        EntryList,
        EntryListProps { entries: vec![entry(1, "alpha", false), entry(2, "beta", true)] },
    );
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
    pretty_assertions::assert_eq!(actual, expected);
}
```

Select by `data-testid` written in `rsx!` (`"data-testid": "entry-{entry.id}"`), never by class, DOM
structure or translated text. Put the tests in `#[cfg(test)] mod tests` beside the component so the
crate's own tests cover it.

**Control: mutate the production line, never the assertions.** The test must go red once before you
trust it. Changing only the production `open_label` branch (`1 => "1 items left"`) failed 2 of the 4 `ui`
tests (`open_label_pluralizes`, `entry_list_renders_items_and_remaining_count`) and left 2 green;
changing every occurrence of the label string, assertions included, kept all four green `[re-run]`. The
string appears in both the code and the expectations, so a find-and-replace proves nothing.

Per-item assertions matter too. With only `contains(r#"class="done""#)`, inverting the production branch
`class: if entry.done { "done" } else { "open" }` swaps the two classes and the test stays green, so
the control cannot fail: the archive's original assertion cannot fail it (`logs/verify-11.log`, H5)
`[re-run]`. Assert the testid and class together on a specific item (`data-testid="entry-2"
class="done"`, and `entry-1` with `class="open"`): the same inversion in production code then fails
`entry_list_renders_items_and_remaining_count` (3 passed, 1 failed, exit 100; `logs/verify-11.log`, H4)
`[re-run]`. Deleting the `data-testid` from the markup is a second control `[derived]`.

## What belongs here

| Concern | Layer |
|---|---|
| Markup for each state (empty, loading, error, populated), conditional classes, `data-testid` presence, effects that derive state from props | here |
| Logic pulled out of components (formatting, filtering, validation, reducers); `open_label(&[Entry])` is tested directly | plain unit or property tests (`rust-test-logic`) |
| Real events, focus, layout, CSS, routing, anything over the network | browser test: not covered here yet |

A VirtualDom rendered to a string cannot click `[zip]`. Two partial answers exist, neither exercised
here: feeding events by hand through `VirtualDom::handle_event` (signature in `dioxus-07`'s
`references/renderer.md`), and the headless `dioxus-native-dom` crate, which runs a VirtualDom with no
window and can inspect layout (`blitz-native`'s `references/embedding.md`; it has no JavaScript and uses
Stylo and Taffy, not a browser engine). Neither replaces a browser test for a critical flow
`[unverified: not run here]`.

## Async data

Provide fake services through context (`use_context_provider`) whose futures are ready immediately;
the flush loop then renders the loaded state. Never call a real API from a component test, and never
add a sleep to wait for one. For components that suspend, `VirtualDom::wait_for_suspense().await` is
the SSR-style wait (`dioxus-07` `references/renderer.md`) `[derived]`; the source's example file
exercises neither pattern `[zip]`.

## Custom hooks

Follow the hook-testing harness in the official 0.7 testing guide
(`https://dioxuslabs.com/learn/0.7/guides/testing/web`): drive the VirtualDom manually and assert at
each render generation `[zip]`. There is no stable component-test crate: `dioxus-test` is experimental,
so do not invent a `dioxus_test::` API (`dioxus-07` `references/tooling.md` section 7).

## Snapshots

For large markup, `insta::assert_snapshot!(html)` instead of many `contains` checks. The source
recommends it in prose only: no validated crate depends on `insta` `[zip]`. Review
every new or changed snapshot before accepting it; a snapshot accepted without reading is a regenerated
expectation, not a test. `cargo insta review` is the review step
`[unverified: cargo-insta not installed]`; installing it is the user's decision (propose and wait).
