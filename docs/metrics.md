# Metric definitions

All evtap metrics cover only the current in-memory session. Results are descriptive signals, not ergonomic or medical diagnoses.

## Event terminology

- **Press:** the initial physical transition from up to down.
- **Release:** the physical transition from down to up.
- **Repeat:** an automatic event generated while a key remains held.
- **Text:** the UTF-8 result produced by the configured XKB state for an event.

Unless stated otherwise, automatic repeats are excluded. Timing samples use timestamps supplied with Linux input events. Samples with reversed timestamps are discarded rather than treated as negative durations.

## Total key presses

Counts every physical press from the selected keyboard. Releases and automatic repeats do not increase the count.

This is a session activity counter, not a count of emitted characters. Modifier, navigation, and function keys count as presses.

## Key usage

Groups physical presses by Linux key identity and ranks them from most to least frequent. Automatic repeats are excluded.

This table is not a physical keyboard heatmap. It has no key geometry, finger assignment, or layout drawing. A future visualization can consume the same physical identities without changing this metric's semantics.

## Correction signals

Maintains a bounded in-memory history of the ten most recent text fragments produced by key events.

When Backspace is pressed or repeated:

1. the most recent text fragment is removed from that history;
2. its deletion count increases;
3. if a text-producing event follows, evtap records an inferred `deleted → typed` correction.

Important limitations:

- A deletion is not necessarily a mistake; it may be intentional editing.
- The next typed text is not necessarily a replacement for the deleted text.
- Navigation, selection, Delete, mouse edits, application behavior, and input-method composition can invalidate the inference.
- Only the bounded recent history is available, so older or application-external context is unknown.

For these reasons the UI calls these values **correction signals**, not an error rate or a true confusion matrix.

## Flight time

Measures the interval from any key release to the next text-producing physical press. Samples of two seconds or more are discarded as breaks rather than typing flow. Automatic repeats are ignored.

Results are grouped by the text produced by the destination key and sorted by the highest average duration. Each row includes its sample count. A high value can indicate hesitation or reach difficulty, but it can also reflect shortcuts, punctuation, editing, or limited data.

## Dwell time

Measures the interval between the physical press and release of a text-producing key. Automatic repeats do not add samples.

The text label is captured on press, so releasing a modifier before releasing the character key does not relabel the sample. Results are grouped by produced text and sorted by highest average duration, with sample counts.

## Bigram speed

Measures press-to-press time between consecutive text-producing physical presses. Samples of two seconds or more are discarded, and automatic repeats are ignored.

A pair appears only after at least three samples. The report shows the five fastest and five slowest average pairs, including sample counts. The labels represent produced text rather than physical key positions, so the configured XKB layout affects them.

## Reset and lifecycle

**Stop listening** ends capture but preserves the aggregates for inspection. **Reset session** clears every aggregate and transient metric buffer. Closing evtap discards the session.
