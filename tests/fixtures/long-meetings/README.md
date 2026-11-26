# Synthetic long-meeting fixtures

The long-meeting notes tests do not use committed transcript blobs or real meeting
content. `src/core/notes/long_meetings_tests.rs` builds every fixture at test run
time.

Each case is a synthetic 1, 2 or 3 hour meeting with three joined recording
parts. Pauses are empty wall-clock spans between those parts. The generator
places a unique fact and its transcript timestamp near the start, middle and
final five minutes, then varies the words per row across sparse, conversation
and dense densities. Dense 2 and 3 hour cases intentionally exceed the current
notes transcript character cap so the regression test can prove what happens to
the tail.

The text is ASCII mixed with stable Unicode probes in French, Hindi and Telugu,
plus one emoji. Speaker rows alternate between microphone and system sources.
All names, facts and tokens are synthetic. This directory holds documentation
only; large generated transcripts stay in test memory and temporary test
directories.
