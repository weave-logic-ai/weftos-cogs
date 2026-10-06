// ADR-104 guide bundles copied from weftos-cog-market/catalog/guides.
// Each value is the JSON a running cog serves at GET /guide.

import bridge from "../guides/bridge.json";
import hlkAs201 from "../guides/hlk-as201.json";
import ld2450 from "../guides/ld2450-radar.json";
import mentraLive from "../guides/mentra-live.json";
import rd03e from "../guides/rd-03e.json";
import sen0213 from "../guides/sen0213-ecg.json";
import sen0628 from "../guides/sen0628-tof.json";
import soundDetect from "../guides/sound-detect.json";

export const GUIDES: Record<string, unknown> = {
  bridge: bridge as unknown,
  "hlk-as201": hlkAs201 as unknown,
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
