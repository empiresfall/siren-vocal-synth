import { useEffect } from "react";
import { useStore } from "../store.ts";
import { DEMO_SONGS, ENGINES } from "../../wasm/engine.ts";
import { ensureDictionary } from "../singing.ts";
import { closeBand, playBand, ROLES, stopBand, useBand, type Role } from "../band.ts";
import { bodyLabel } from "../body.ts";
import { Chip, Screen } from "./Screen.tsx";
import { Segmented, Slider } from "./Controls.tsx";
import { SpectrumView } from "./SpectrumView.tsx";
import { Spectrogram } from "./Spectrogram.tsx";
import type { BodyName } from "../../worklet/protocol.ts";
import type { EngineKind } from "../../wasm/types.ts";

export const ENGINE_LABEL: Record<EngineKind, string> = {
  airway: "Airway",
  waveguide: "Wave",
  formant: "Formant",
  neural: "Neural",
};
const PART_ENGINE_LABEL: Record<EngineKind, string> = {
  airway: "Air",
  waveguide: "Wave",
  formant: "Form",
  neural: "Neur",
};
const PART_ENGINE_OPTIONS = ENGINES.map((e) => ({ value: e, label: PART_ENGINE_LABEL[e] }));

function Parts() {
  const parts = useBand((s) => s.parts);
  const setPart = useBand((s) => s.setPart);
  const body = useStore((s) => s.body);
  const kind = useStore((s) => s.engineKind);
  return (
    <div className="band-parts">
      <div className="band-part lead">
        <span className="band-name">Lead</span>
        <span className="band-note">
          The head on screen: {bodyLabel(body)}, {ENGINE_LABEL[kind]}. Change it in the top bar.
        </span>
      </div>
      {parts.map((p, i) => (
        <div key={i} className={`band-part${p.on ? "" : " off"}`}>
          <Chip on={p.on} onClick={() => setPart(i, { on: !p.on })} title={p.on ? "Switch this part off" : "Switch this part on"}>
            Part {i + 2}
          </Chip>
          <Segmented<Role> label={`Part ${i + 2} sings`} value={p.role} onChange={(role) => setPart(i, { role })} options={ROLES.map((r) => ({ value: r.value, label: r.label }))} />
          <Segmented<BodyName>
            label={`Part ${i + 2} body`}
            value={p.body}
            onChange={(b) => setPart(i, { body: b })}
            options={[
              { value: "male", label: "M" },
              { value: "female", label: "F" },
              { value: "child", label: "C" },
            ]}
          />
          <Segmented<EngineKind> label={`Part ${i + 2} engine`} value={p.engine} onChange={(e) => setPart(i, { engine: e })} options={PART_ENGINE_OPTIONS} />
          <Slider label="Level" value={p.level} min={0} max={1} onChange={(v) => setPart(i, { level: v })} format={(v) => `${Math.round(v * 100)} %`} />
        </div>
      ))}
    </div>
  );
}

/** Band mode's right column: the song and its words, the parts, and what the band sounds like. */
export function BandRight() {
  useEffect(ensureDictionary, []);
  // leaving the mode stops the band and drops its extra voices
  useEffect(() => closeBand, []);
  const song = useBand((s) => s.song);
  const setSong = useBand((s) => s.setSong);
  const lyrics = useBand((s) => s.lyrics);
  const setLyrics = useBand((s) => s.setLyrics);
  const playing = useBand((s) => s.playing);
  const loop = useBand((s) => s.loop);
  const setLoop = useBand((s) => s.setLoop);
  const cur = DEMO_SONGS.find((s) => s.name === song) ?? DEMO_SONGS[0];
  return (
    <div className="right right-band">
      <Screen
        className="s-song"
        title="Song"
        sub={`${cur.bpm} bpm`}
        tools={
          <>
            <Chip on={loop} onClick={() => setLoop(!loop)}>
              Loop
            </Chip>
            <Chip on={playing} tone="amber" onClick={() => (playing ? stopBand() : void playBand())}>
              {playing ? "Stop the band" : "Play the band"}
            </Chip>
          </>
        }
      >
        <div className="band-song">
          <div className="demo-songs" role="group" aria-label="Tune">
            {DEMO_SONGS.map((s) => (
              <Chip key={s.name} on={s.name === song} onClick={() => setSong(s.name)}>
                {s.name}
              </Chip>
            ))}
          </div>
          <textarea
            value={lyrics}
            onChange={(e) => setLyrics(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
            rows={2}
            aria-label="Your own words for the tune"
            placeholder={`Your own words for the tune, in English or Russian (empty: “${cur.lyrics.slice(0, 40)}…”)`}
          />
        </div>
      </Screen>
      <Screen className="s-parts" title="Parts" sub="Each part is its own voice">
        <Parts />
      </Screen>
      <Screen className="s-spec" title="Spectrum" sub="The lead's resonances">
        <SpectrumView />
      </Screen>
      <Screen className="s-gram" title="Spectrogram" sub="Everyone together">
        <Spectrogram />
      </Screen>
    </div>
  );
}
