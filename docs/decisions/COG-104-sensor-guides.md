# COG-104: A sensor guide ships with the cog

- **Status:** Accepted
- **Decision date:** 2026-10-01 (Owner / platform, WeftOS ADR-104)
- **Recorded here:** 2026-10-06, as the public successor. This file does not renumber or amend the WeftOS record.
- **Predecessor:** WeftOS ADR-104, *Sensor guides ship with sensor cogs*
- **Audience:** someone writing or installing a sensor cog in this repository
- **Question:** where does the hook-up guide live, and how does a client get the copy that matches the cog it is talking to?
- **Rests on:** WeftOS ADR-104; `crates/weftos-sensor-guide` and `crates/weftos-cog-companion`; `src/cogs/sen0213-ecg/src/guide.rs` and `src/cogs/sen0213-ecg/src/export.rs` (read at `056029567eb8`)

## Context

A sensor cog is useful only after someone has wired the sensor, placed it, enabled the bus, installed the cog, and checked the signal. That knowledge used to live in READMEs, which are not at the bench next to a live signal. The guide and the cog have to be the same version, so the guide is part of the cog rather than a page maintained beside it.

## Decision

A sensor guide ships with the cog.

1. **Every sensor cog carries its guide** in `guide/` next to the cog: `guide.toml` (schema 1) and one CommonMark page per topic. `guide.toml` holds the page order, checklist links, and the data the diagrams are drawn from (header pins, parts and wires, placements, signal flow, and a grid for a matrix sensor). The first `# ` line of a page is the title and the first `> ` line is the summary. A fenced block whose info string is `diagram` and whose body is `header`, `wiring`, `placements`, `flow`, or `grid` embeds that diagram.
2. **The cog serves that guide** at `GET /guide` on its export port, as `{"toml": "...", "pages": {id: markdown}}`, compiled in with the cog. A client therefore sees the guide that matches the installed cog, not a separately published copy. The cog's tests check that the embedded page list matches `guide.toml`.
3. **`weftos-sensor-guide` is the renderer.** It validates a bundle and draws it in egui, natively and in the browser: page navigation, Markdown, and diagrams painted from the data. Validation rejects a link to a missing page, a wire endpoint that is not a declared part pin, a header pin outside 1–40 or listed as both used and avoided, a pad outside 0..1, and a diagram fence with no data behind it. `guide-check` runs that validation over a cog's `guide/` directory.
4. **Companion apps share one shell, `weftos-cog-companion`.** A sensor app implements `SensorApp` (live view, checklist steps, calibration, extra export polling). The shell provides the connection bar, start / stop / test-run, the hook-up checklist with a "?" from each step into the guide, settings from the cog manifest, and the Guide tab. The app owns the live checks. The guide owns the explanation. Links from step ids to pages are `[links]` in `guide.toml`.
5. **Authoring.** Facts match the cog and its decision record; anything unverified is marked. Trust pin labels over wire colours. Medical-adjacent sensors set `medical = false` and carry a safety page. The format is versioned: a renderer rejects an unknown schema rather than guessing.

## Consequences

- A new sensor cog gets a Guide tab, diagrams, and validation by writing `guide.toml` and Markdown. It does not need a new renderer.
- The same Markdown renders on a docs site. Diagrams stay data, so they can be checked against the wiring tables.
- The guide travels inside the cog binary. `sen0213-ecg` serves its compiled pages at `GET /guide`. This record is the rule. It is not a census of which cogs in this tree already have a `guide/` directory.

## What stays in WeftOS

WeftOS ADR-104 remains the predecessor record of the decision, including its date and the argument. This file is the public statement of that decision for this repository. It does not move OS or runtime decisions.

## Alternatives considered

- **Compile the guide into each companion app.** The text then drifts from the cog version, and no other client can read it.
- **Static HTML only.** It does not render in the companion shell, and the checklist cannot link into it.
- **Diagrams as images.** They cannot be checked against the wiring data.
