//! Orderings and state combinations the hand-written cases never visit.
//!
//! The scenario fixture replays every event variant, but each one only along
//! the path its case was written for. That proves breadth, not that the twins
//! agree when the same events arrive in another order or land on a state no
//! case reached: a stale answer after a newer request, a reload on top of a
//! half-finished flow, the same event delivered twice.
//!
//! This builds those sequences mechanically from the existing cases, with a
//! fixed-seed generator so the fixture is byte-identical on every run, and
//! records what `reduce` does after every step. The frontend replays them in
//! `reducer.orderings.conformance.test.ts`.
//!
//! Every event is one a case already sends, so every event deserialises; the
//! orders and starting states are deliberately the ones nobody planned for.
//!
//! The slice after each step is stored as a diff against the previous step
//! (`set` and `unset` paths into the slice), not in full. A recorded slice is
//! up to several KB and most events change one field of it, so storing every
//! step whole would cost tens of MB for the same information. Diffs repeat a
//! great deal (the same event on the same field), so they are stored once and
//! steps name them by index, as they do events. The diff is lossless: the
//! generator applies it back and asserts it reproduces the state, and the
//! frontend compares the *whole* reconstructed slice.

use super::{cases, event_slice};
use faf_domain::{reduce, AppEvent, AppState};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

/// Changing this reshuffles every sequence. That is fine, but do it on purpose:
/// a divergence a seed found stays reproducible only under that seed.
const SEED: u64 = 0x0FAF_2026_0C0F_F1E5;

/// Sequences per operation for a slice with `sources` cases: richer slices get
/// more, up to a cap that keeps the fixture a few MB.
fn per_operation(sources: usize) -> usize {
    (6 + 3 * sources).min(40)
}

/// Random walks per sequence of every other operation.
const WALKS_PER_OPERATION: usize = 4;

/// Longest random walk, in events.
const MAX_WALK: usize = 20;

/// The written fixture must stay below this. Sequences are cheap; distinct
/// starting states are what cost space.
const MAX_FIXTURE_BYTES: usize = 4 * 1024 * 1024;

/// SplitMix64: tiny, well-distributed, and written out here so the sequences
/// do not depend on a crate's algorithm staying the same across versions.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`. The modulo bias is irrelevant at these sizes.
    fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "cannot pick from nothing");
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = self.below(index + 1);
            items.swap(index, other);
        }
    }
}

/// One case's events for one slice, with the state after each.
struct Source {
    name: String,
    events: Vec<AppEvent>,
    /// The checkpoint holding the state after the last event.
    last: usize,
}

/// A state a generated sequence may start from: the default, or any
/// intermediate state of any case for the slice.
struct Checkpoint {
    label: String,
    state: AppState,
}

struct Corpus {
    sources: Vec<Source>,
    checkpoints: Vec<Checkpoint>,
    /// Every distinct event the slice's cases send.
    events: Vec<AppEvent>,
}

struct Plan {
    name: String,
    start: usize,
    events: Vec<AppEvent>,
}

#[derive(Serialize)]
struct OrderingFixture {
    /// The seed, as text so JavaScript does not round it.
    seed: String,
    /// Every event any sequence sends, deduplicated; steps refer by index.
    events: Vec<Value>,
    /// Every slice change any step makes, deduplicated; see [`Diff`].
    diffs: Vec<Value>,
    /// Starting slices, deduplicated; each sequence names one by index.
    starts: Vec<Value>,
    sequences: Vec<Sequence>,
}

#[derive(Serialize)]
struct Sequence {
    name: String,
    /// The `AppState` field every event of the sequence owns.
    slice: String,
    start: usize,
    /// `[event, diff]` index pairs.
    steps: Vec<(usize, usize)>,
}

/// How one step changed the slice: apply `unset`, then `set`, to the previous
/// step's slice to get this step's. A path walks object keys (strings) and
/// array indices (numbers); an empty path is the whole slice. An event that
/// changed nothing has an empty diff.
#[derive(Serialize, Default)]
struct Diff {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    set: Vec<(Vec<Value>, Value)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unset: Vec<Vec<Value>>,
}

fn ordering_fixture() -> OrderingFixture {
    let mut rng = Rng(SEED);
    let mut events = Interner::default();
    let mut diffs = Interner::default();
    let mut starts = Interner::default();
    let mut sequences = Vec::new();

    for (slice, corpus) in corpora() {
        for plan in plans(&corpus, &mut rng) {
            let checkpoint = &corpus.checkpoints[plan.start];
            let mut state = checkpoint.state.clone();
            let mut previous = slice_of(&state, &slice);
            let start = starts.intern(previous.clone());
            let steps = plan
                .events
                .iter()
                .map(|event| {
                    reduce(&mut state, event);
                    let next = slice_of(&state, &slice);
                    let mut change = Diff::default();
                    diff(&previous, &next, &mut Vec::new(), &mut change);
                    let mut rebuilt = previous.clone();
                    apply(&mut rebuilt, &change);
                    assert_eq!(rebuilt, next, "the step diff must reproduce the state");
                    previous = next;
                    let event = serde_json::to_value(event).expect("event must serialise");
                    let change = serde_json::to_value(change).expect("a diff must serialise");
                    (events.intern(canonical(event)), diffs.intern(change))
                })
                .collect();
            sequences.push(Sequence {
                name: format!("{slice}: {}", plan.name),
                slice: slice.clone(),
                start,
                steps,
            });
        }
    }

    OrderingFixture {
        seed: format!("{SEED:#018x}"),
        events: events.values,
        diffs: diffs.values,
        starts: starts.values,
        sequences,
    }
}

/// Each case split by the slice its events own, replayed from the default.
fn corpora() -> BTreeMap<String, Corpus> {
    let mut by_slice: BTreeMap<String, Vec<(String, Vec<AppEvent>)>> = BTreeMap::new();
    for case in cases() {
        let mut per_slice: BTreeMap<String, Vec<AppEvent>> = BTreeMap::new();
        for step in case.steps {
            per_slice
                .entry(event_slice(&step.event))
                .or_default()
                .push(step.event);
        }
        for (slice, events) in per_slice {
            by_slice
                .entry(slice)
                .or_default()
                .push((case.name.clone(), events));
        }
    }

    by_slice
        .into_iter()
        .map(|(slice, raw)| {
            let mut checkpoints = vec![Checkpoint {
                label: "the default state".into(),
                state: AppState::default(),
            }];
            let mut distinct = Interner::default();
            let mut events = Vec::new();
            let sources = raw
                .into_iter()
                .map(|(name, case_events)| {
                    let mut state = AppState::default();
                    for (index, event) in case_events.iter().enumerate() {
                        reduce(&mut state, event);
                        checkpoints.push(Checkpoint {
                            label: format!("`{name}` after step {}", index + 1),
                            state: state.clone(),
                        });
                        let key = canonical(serde_json::to_value(event).expect("serialises"));
                        if distinct.intern(key) == events.len() {
                            events.push(event.clone());
                        }
                    }
                    Source {
                        name,
                        events: case_events,
                        last: checkpoints.len() - 1,
                    }
                })
                .collect();
            (
                slice,
                Corpus {
                    sources,
                    checkpoints,
                    events,
                },
            )
        })
        .collect()
}

fn plans(corpus: &Corpus, rng: &mut Rng) -> Vec<Plan> {
    let count = per_operation(corpus.sources.len());
    let mut plans = Vec::new();

    // Two flows of the same slice racing each other: whichever answer lands
    // first, the other's request may already be stale.
    for _ in 0..count {
        let first = rng.pick(&corpus.sources);
        let second = rng.pick(&corpus.sources);
        let (mut left, mut right) = (first.events.iter(), second.events.iter());
        let (mut left_count, mut right_count) = (first.events.len(), second.events.len());
        let mut events = Vec::new();
        while left_count + right_count > 0 {
            if rng.below(left_count + right_count) < left_count {
                events.extend(left.next().cloned());
                left_count -= 1;
            } else {
                events.extend(right.next().cloned());
                right_count -= 1;
            }
        }
        plans.push(Plan {
            name: format!("`{}` interleaved with `{}`", first.name, second.name),
            start: 0,
            events,
        });
    }

    // Events lost on the way: a reply whose request never arrived, a
    // completion without its start.
    for _ in 0..count {
        let source = rng.pick(&corpus.sources);
        let mut events: Vec<AppEvent> = source
            .events
            .iter()
            .filter(|_| rng.chance(50))
            .cloned()
            .collect();
        if events.is_empty() {
            events.push(rng.pick(&source.events).clone());
        }
        plans.push(Plan {
            name: format!("a subsequence of `{}`", source.name),
            start: 0,
            events,
        });
    }

    // Everything out of order.
    for _ in 0..count {
        let source = rng.pick(&corpus.sources);
        let mut events = source.events.clone();
        rng.shuffle(&mut events);
        plans.push(Plan {
            name: format!("`{}` shuffled", source.name),
            start: 0,
            events,
        });
    }

    // Duplicate delivery: the same event up to three times in a row.
    for _ in 0..count {
        let source = rng.pick(&corpus.sources);
        let mut events = Vec::new();
        for event in &source.events {
            events.push(event.clone());
            let mut copies = 1;
            while copies < 3 && rng.chance(35) {
                events.push(event.clone());
                copies += 1;
            }
        }
        plans.push(Plan {
            name: format!("`{}` with repeats", source.name),
            start: 0,
            events,
        });
    }

    // One whole flow on top of wherever another one stopped, including its
    // own final state (the flow run twice).
    for _ in 0..count {
        let start = rng.below(corpus.checkpoints.len());
        let source = rng.pick(&corpus.sources);
        plans.push(Plan {
            name: format!("`{}` from {}", source.name, corpus.checkpoints[start].label),
            start,
            events: source.events.clone(),
        });
    }

    // Arbitrary events of the slice from an arbitrary reachable state. The
    // most likely to land an event somewhere its arm was never written for,
    // so it gets the largest share.
    for _ in 0..WALKS_PER_OPERATION * count {
        let start = rng.below(corpus.checkpoints.len());
        let length = 1 + rng.below(MAX_WALK);
        let events: Vec<AppEvent> = (0..length)
            .map(|_| rng.pick(&corpus.events).clone())
            .collect();
        plans.push(Plan {
            name: format!(
                "{length} random events from {}",
                corpus.checkpoints[start].label
            ),
            start,
            events,
        });
    }

    // The final state of every case followed by every one of its own steps
    // again in reverse: answers arriving newest first.
    for source in &corpus.sources {
        let mut events = source.events.clone();
        events.reverse();
        plans.push(Plan {
            name: format!("`{}` reversed from its own end", source.name),
            start: source.last,
            events,
        });
    }

    plans
}

/// Maps each `AppState` field to its serialised name, so a step serialises
/// only the slice it changed: serialising the whole state for each of tens of
/// thousands of steps made the generator take most of a minute.
/// `the_slice_table_lists_every_slice` keeps the table in step with the type.
macro_rules! slice_table {
    ($($field:ident => $name:literal),* $(,)?) => {
        const SLICE_NAMES: &[&str] = &[$($name),*];

        /// The slice an event owns, with object keys in a fixed order so a
        /// `HashMap` in the state cannot make two runs write different files.
        fn slice_of(state: &AppState, slice: &str) -> Value {
            let value = match slice {
                $($name => serde_json::to_value(&state.$field),)*
                other => panic!("`{other}` is not in the slice table"),
            };
            canonical(value.expect("a slice must serialise"))
        }
    };
}

slice_table! {
    session => "session",
    install => "install",
    auth => "auth",
    nav => "nav",
    notifications => "notifications",
    chat => "chat",
    clan => "clan",
    coop => "coop",
    events => "events",
    lobby => "lobby",
    replays => "replays",
    maps => "maps",
    map_generator => "mapGenerator",
    mods => "mods",
    leaderboard => "leaderboard",
    player_card => "playerCard",
    reporting => "reporting",
    reviews => "reviews",
    social => "social",
    streams => "streams",
    tourney => "tourney",
    training => "training",
    tutorials => "tutorials",
    uploads => "uploads",
    galactic_war => "galacticWar",
    guides => "guides",
    client_update => "clientUpdate",
    mac_setup => "macSetup",
    settings => "settings",
    changelog => "changelog",
}

#[test]
fn the_slice_table_lists_every_slice() {
    let state = serde_json::to_value(AppState::default()).expect("state must serialise");
    let mut declared: Vec<&str> = state
        .as_object()
        .expect("state is an object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut listed = SLICE_NAMES.to_vec();
    declared.sort_unstable();
    listed.sort_unstable();
    assert_eq!(
        listed, declared,
        "update `slice_table!` to match `AppState`"
    );
    // And each entry reads the field its name says, checked on states the
    // cases left behind: defaults alone could not tell two slices apart.
    for (slice, corpus) in corpora() {
        for source in &corpus.sources {
            let state = &corpus.checkpoints[source.last].state;
            let whole = serde_json::to_value(state).expect("state must serialise");
            assert_eq!(slice_of(state, &slice), canonical(whole[&slice].clone()));
        }
    }
}

fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(sorted.into_iter().collect::<Map<String, Value>>())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// Records how `before` became `after`. Objects and equal-length arrays are
/// walked; anything else that changed is replaced whole at its path.
fn diff(before: &Value, after: &Value, path: &mut Vec<Value>, step: &mut Diff) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            for key in old.keys().filter(|key| !new.contains_key(*key)) {
                let mut removed = path.clone();
                removed.push(Value::String(key.clone()));
                step.unset.push(removed);
            }
            for (key, value) in new {
                path.push(Value::String(key.clone()));
                match old.get(key) {
                    Some(previous) => diff(previous, value, path, step),
                    None => step.set.push((path.clone(), value.clone())),
                }
                path.pop();
            }
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
            for (index, (previous, value)) in old.iter().zip(new).enumerate() {
                path.push(Value::from(index));
                diff(previous, value, path, step);
                path.pop();
            }
        }
        _ => step.set.push((path.clone(), after.clone())),
    }
}

/// The inverse of [`diff`], as the frontend runs it.
fn apply(slice: &mut Value, step: &Diff) {
    for path in &step.unset {
        let (last, parent) = path.split_last().expect("an unset path is never empty");
        let key = last.as_str().expect("only object keys are unset");
        locate(slice, parent)
            .as_object_mut()
            .expect("an unset parent is an object")
            .remove(key);
    }
    for (path, value) in &step.set {
        match path.split_last() {
            None => *slice = value.clone(),
            Some((last, parent)) => {
                let target = locate(slice, parent);
                match last {
                    Value::String(key) => {
                        target
                            .as_object_mut()
                            .expect("a keyed parent is an object")
                            .insert(key.clone(), value.clone());
                    }
                    index => {
                        let index = index.as_u64().expect("a path is keys and indices") as usize;
                        target
                            .as_array_mut()
                            .expect("an indexed parent is an array")[index] = value.clone();
                    }
                }
            }
        }
    }
}

fn locate<'a>(mut value: &'a mut Value, path: &[Value]) -> &'a mut Value {
    for segment in path {
        value = match segment {
            Value::String(key) => value.get_mut(key.as_str()),
            index => value.get_mut(index.as_u64().expect("an index") as usize),
        }
        .expect("a diff path exists in the state it was taken from");
    }
    value
}

/// Deduplicates values by their canonical JSON text.
#[derive(Default)]
struct Interner {
    values: Vec<Value>,
    index: HashMap<String, usize>,
}

impl Interner {
    fn intern(&mut self, value: Value) -> usize {
        let key = value.to_string();
        if let Some(&index) = self.index.get(&key) {
            return index;
        }
        self.values.push(value);
        self.index.insert(key, self.values.len() - 1);
        self.values.len() - 1
    }
}

/// Writes `reducer-conformance-orderings.json` next to the scenario fixture.
///
/// Built twice and compared first: a reducer whose result depended on a
/// `HashMap`'s iteration order would make the recorded expectations change
/// between runs, and that must fail here rather than flicker in the frontend.
#[test]
fn writes_the_frontend_ordering_fixture() {
    let fixture = ordering_fixture();
    let covered: std::collections::BTreeSet<&str> = fixture
        .sequences
        .iter()
        .map(|sequence| sequence.slice.as_str())
        .collect();
    for slice in SLICE_NAMES {
        assert!(
            covered.contains(slice),
            "`{slice}` has no ordering sequences"
        );
    }
    assert!(
        fixture
            .sequences
            .iter()
            .all(|sequence| !sequence.steps.is_empty()),
        "an empty sequence would pass while proving nothing"
    );

    let json = serde_json::to_string(&fixture).expect("the fixture must serialise");
    let again = serde_json::to_string(&ordering_fixture()).expect("the fixture must serialise");
    assert!(json == again, "the ordering fixture is not deterministic");
    assert!(
        json.len() < MAX_FIXTURE_BYTES,
        "the ordering fixture is {} bytes; lower `per_operation` or `MAX_WALK`",
        json.len()
    );

    let target =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/store/__fixtures__");
    std::fs::create_dir_all(&target).expect("could not create the fixture directory");
    std::fs::write(
        target.join("reducer-conformance-orderings.json"),
        format!("{json}\n"),
    )
    .expect("could not write the fixture");
}
