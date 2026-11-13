import { useStore, HARMONIES } from "../store.ts";
import { FX, GOLDEN } from "../../wasm/engine.ts";
import { Encoder } from "./Encoder.tsx";
import { Chip, Screen } from "./Screen.tsx";
import { setMicFollow, useMic } from "../micFollow.ts";

const pct = (v: number) => ({ value: String(Math.round(v * 100)), unit: "%" });

/** Record and save, in the rack's header. */
function Recorder() {
  const recording = useStore((s) => s.recording);
  const setRecording = useStore((s) => s.setRecording);
  const take = useStore((s) => s.take);
  return (
    <>
      {take && !recording && (
        <a className="chip take" href={take.url} download={take.name} title="Save the last take as a WAV file">
          Save WAV
        </a>
      )}
      <Chip on={recording} tone="slot1" onClick={() => setRecording(!recording)} title="Record what you hear, effects included">
        {recording ? "Stop" : "Rec"}
      </Chip>
    </>
  );
}

/**
 * The rack under the wall, in every mode: the eight effects (a chip each: press to open it,
 * press again to switch it), the open effect's four controls, and the voice's extras: the golden
 * voice, the harmony voices, and the microphone the head can follow.
 */
export function Rack() {
  const fx = useStore((s) => s.fx);
  const selectFx = useStore((s) => s.selectFx);
  const setFxOn = useStore((s) => s.setFxOn);
  const setFxParam = useStore((s) => s.setFxParam);
  const golden = useStore((s) => s.source.golden);
  const setSource = useStore((s) => s.setSource);
  const harmony = useStore((s) => s.harmony);
  const toggleHarmony = useStore((s) => s.toggleHarmony);
  const human = useStore((s) => s.mode !== "bestiary");
  const play = useStore((s) => s.mode === "play");
  const mic = useMic((s) => s.on);
  const micError = useMic((s) => s.error);
  const heard = useMic((s) => s.heard);
  const sel = FX[fx.selected];
  const lit = fx.on.filter(Boolean).length;
  return (
    <Screen className="s-rack" title="Rack" sub={lit ? `${lit} effect${lit > 1 ? "s" : ""} on` : "Effects off"} tools={<Recorder />}>
      <div className="rack">
        <div className="rack-fx">
          <div className="rack-chips" role="group" aria-label="Effects">
            {FX.map((f, i) => (
              <Chip
                key={f.id}
                on={fx.on[i]}
                className={i === fx.selected ? "open" : ""}
                title={`${f.name}: ${f.about}`}
                onClick={() => {
                  // the first press opens an effect (and switches it on); pressing the open one switches it
                  if (i !== fx.selected) {
                    selectFx(i);
                    if (!fx.on[i]) setFxOn(i, true);
                  } else setFxOn(i, !fx.on[i]);
                }}
              >
                {f.name}
              </Chip>
            ))}
          </div>
          <div className="enc-row cols-4" role="group" aria-label={`${sel.name} controls`}>
            {sel.params.map((name, k) => (
              <Encoder key={`${sel.id}-${name}`} slot={k} label={name} value={fx.params[fx.selected][k]} min={0} max={1} onChange={(v) => setFxParam(fx.selected, k, v)} format={pct} />
            ))}
          </div>
        </div>
        {human && (
          <div className="rack-voice">
            <Encoder
              slot={3}
              label="Golden"
              value={golden}
              min={0}
              max={1}
              onChange={(v) => setSource({ golden: v })}
              format={(v) => (v < 0.005 ? { value: "Off", unit: "" } : { value: String(Math.round(v * 100)), unit: `% · ±${(GOLDEN.cents * v).toFixed(1)} ct` })}
            />
            <div className="rack-extras">
              <div className="rack-chips" role="group" aria-label="Harmony voices">
                {HARMONIES.map((h) => (
                  <Chip key={h.id} on={harmony.includes(h.id)} title={h.title} onClick={() => toggleHarmony(h.id)}>
                    {h.label}
                  </Chip>
                ))}
              </div>
              {play && (
                <div className="rack-chips">
                  <Chip on={mic} tone="amber" onClick={() => void setMicFollow(!mic)} title="The head follows your voice at the microphone: its pitch and its vowel. The microphone is only analysed, never played back.">
                    {mic ? "Following mic" : "Follow mic"}
                  </Chip>
                  {mic && <span className="rack-heard">{heard.f0 ? `${Math.round(heard.f0)} Hz` : "…"} {heard.vowel ? `/${heard.vowel}/` : ""}</span>}
                  {micError && <span className="rack-heard error">Microphone: {micError}</span>}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </Screen>
  );
}
