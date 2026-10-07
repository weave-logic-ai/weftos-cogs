// ADR-104 guide bundles. The canonical files live in cog-market. Each value is the JSON
// a running cog serves at GET /guide.

import bridge from "../../../crates/cog-market/catalog/guides/bridge.json";
import hlkAs201 from "../../../crates/cog-market/catalog/guides/hlk-as201.json";
import ld1040c from "../../../crates/cog-market/catalog/guides/ld1040c-motion.json";
import ld2450 from "../../../crates/cog-market/catalog/guides/ld2450-radar.json";
import mentraLive from "../../../crates/cog-market/catalog/guides/mentra-live.json";
import rd03e from "../../../crates/cog-market/catalog/guides/rd-03e.json";
import sen0213 from "../../../crates/cog-market/catalog/guides/sen0213-ecg.json";
import sen0628 from "../../../crates/cog-market/catalog/guides/sen0628-tof.json";
import soundDetect from "../../../crates/cog-market/catalog/guides/sound-detect.json";

export const GUIDES: Record<string, unknown> = {
  bridge: bridge as unknown,
  "hlk-as201": hlkAs201 as unknown,
  "ld1040c-motion": ld1040c as unknown,
  "ld2450-radar": ld2450 as unknown,
  "mentra-live": mentraLive as unknown,
  "rd-03e": rd03e as unknown,
  "sen0213-ecg": sen0213 as unknown,
  "sen0628-tof": sen0628 as unknown,
  "sound-detect": soundDetect as unknown,
};

export function guideFor(id: string): unknown | null {
  return Object.prototype.hasOwnProperty.call(GUIDES, id) ? GUIDES[id] : null;
}
