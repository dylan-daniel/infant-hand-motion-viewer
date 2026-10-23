# Grasp events CSV

The standard CSV the hand motion viewer loads for grasp annotations: one row per trial, giving the object, the hand(s)
used for the grasp, the completing grip, and the times of three events (E2 reach onset, E3 first contact, E4 grip
established). It is derived from Grace's event-timing coding (batches 1 and 2) but is not tied to it: anyone can write a
file in this format. The converted batches 1 and 2 are at
`/HDD16TB/infant_grasp_workspace_outputs/dataset/grasp_events_grace_batches_1_2.csv` (516 rows, 31 columns).

## File rules

- UTF-8, comma-separated, one header row first, one row per trial, quoted where a cell contains a comma.
- All text is lowercase `snake_case`. Booleans are `true` / `false`. **A blank cell means not applicable or unknown**;
  it is never a sentinel number (no `8888`/`9999`).
- Rows are keyed by `subject` + `trial`. A duplicate key within one file is an error; the file is rejected.
- `subject` and `trial` are required. Every other column is optional: a missing column reads as blank everywhere.
  Unknown extra columns are ignored.
- Event times are **seconds from the start of the trial video**, three decimals. The viewer converts them to frames
  with the video's frame rate. Times are not guaranteed to be in order (E2 <= E3 <= E4); do not assume it.

## Columns

| column | values | notes |
|---|---|---|
| `subject` | e.g. `AE45` | must equal the `.hexport` metadata `subject` |
| `trial` | e.g. `T1_BabyView` | must equal the `.hexport` metadata `trial` |
| `object_size` | `small`, `medium`, `large` | |
| `object_shape` | `cube`, `sphere` | |
| `object_color` | `red`, `yellow` | |
| `contact_before_grip` | `true`, `false` | |
| `left_hand_used`, `right_hand_used` | `true`, `false` | both can be true (15 trials) or both false (2 trials) |
| `left_grip_type`, `right_grip_type` | `power`, `precision` | blank when that hand was not used |
| `left_thumb`, `left_index`, `left_middle`, `left_ring`, `left_little` (and `right_*`) | `first`, `interior`, `tip` | how that digit contacts the object in the completing grip; blank if no contact or hand unused |
| `left_palm_contact`, `right_palm_contact` | `no`, `palm`, `radial`, `ulnar` | `no` is a real value, blank is unknown or hand unused |
| `left_thumb_opposition`, `right_thumb_opposition` | `true`, `false` | |
| `e2`, `e3`, `e4` | seconds, or blank | reach onset, first contact on the object, grip established; blank if not observable or did not occur |
| `e2_certainty`, `e3_certainty`, `e4_certainty` | `clear`, `probable`, `inferred`, `not_observable` | `not_observable` goes with a blank time |
| `contact_status` | `continuous_contact`, `broke_and_recontacted` | |

Case note: identifiers are written exactly as the `.hexport` header has them (`AE45`, `T1_BabyView`), not lowercased,
because the viewer joins on them as strings. Everything else is lowercase.

## Mapping from Grace's files

- Header is on row 5; rows 1-4 are titles and notes. `Baby ID` -> `subject`; `Trial` `1` -> `T1_BabyView`.
- `Yes`/`No` -> `true`/`false`; other values lowercased with spaces turned into underscores.
- `E2 reach onset (s)`, `E3 first contact (s)`, `E4 grip established (s)` -> `e2`, `e3`, `e4`. A value of `8888` or
  `9999` becomes blank and its certainty stays `not_observable` (or blank where Grace left it blank).
- **Rows with `Longer primary clip needed? == Yes` are dropped** (39 trials: 12 in batch 1, 27 in batch 2). Grace timed
  them on `<id>_BabyView_Longer.mp4` clips we do not have, so their times do not line up with our videos.
- Not carried over: batch, backup view fields, camera views, challenge reason, scoring flag, moving-object flags,
  event-timing note, and Grace's prior (Seattle) reference columns.

## Several files

The viewer can load more than one CSV (local or remote). Files are ordered in a list, and the file higher in the list
has higher priority. For a trial present in several files, **the whole row of the highest-priority file wins**: it
replaces the lower rows entirely, blank cells included, so a corrected row is never merged cell by cell with an old one.
Trials present in only one file are used as they are.

## What the data looks like (batches 1 and 2)

516 rows from 555, 48 infants. Blank e2, e3 and e4 appear in 28, 35 and 28 rows (not observable, plus one trial where
E4 did not occur). 15 trials use both hands and 2 use neither (KA63 T5, QV80 T9). 488 rows match one of the 547 exports. The other 28 have
no export: 24 are from BM56 and CM25, and 27 of the 28 have a blank e2 (Grace had no baby-view video, so the times are not
observable). The remaining four rows (JK95 two, HZ19 one, VX41 one) have times but no matching export; not investigated.
