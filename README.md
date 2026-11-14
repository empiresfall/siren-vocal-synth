# SIREN

[![Live Demo on Vercel](https://img.shields.io/badge/Live_Synth-siren--vocal--synth.vercel.app-ffd24a?style=for-the-badge&logo=vercel&logoColor=black)](https://siren-vocal-synth.vercel.app)
[![Instagram @vibecoding_news](https://img.shields.io/badge/Instagram-%40vibecoding__news-E4405F?style=for-the-badge&logo=instagram&logoColor=white)](https://www.instagram.com/vibecoding_news/)
[![Ableton VST3 Release](https://img.shields.io/badge/macOS_VST3_%2F_CLAP-Download_Release-4fb6f2?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/empiresfall/siren-vocal-synth/releases/latest)

- **Live Web App (Vercel):** https://siren-vocal-synth.vercel.app
- **Instagram:** https://www.instagram.com/vibecoding_news/ (`@vibecoding_news`)
- **GitHub Repository:** https://github.com/empiresfall/siren-vocal-synth

> **SIREN** is a multi-engine vocal, choir, beatbox, and creature synthesizer (WebAssembly +
> native Ableton Live VST3/CLAP plugin), built for [**Vibe Coding News**](https://www.instagram.com/vibecoding_news/)
> (`@vibecoding_news`) and forked from [CHIMERA](https://github.com/jamesondh/chimera-vocal-synth)
> by Jameson Hodge (MIT) at commit `e33221d`. [What SIREN adds](#what-siren-adds) is listed below;
> the original's physical airway voice is preserved bit-for-bit (its golden tests pass untouched).

A physical-modeling voice synthesizer in the browser and DAW. Every sound comes from a simulated
airway (lungs, vocal folds or syrinx, a branched vocal tract, lips, nose), not from samples.
The engine is Rust, compiled to WebAssembly for an AudioWorklet and to native macOS bundles
for the Ableton Live VST3/CLAP plugin, tests, and the `chimera` CLI.

## Vision & scope

1. **One instrument**: a physically modeled voice (lungs, vocal folds, throat, mouth). No
   samples, no neural audio.
2. **The web app is the primary surface**: anyone should be able to make songs in it without
   an LLM's help ([siren-vocal-synth.vercel.app](https://siren-vocal-synth.vercel.app)). Mobile is
   the likely next surface. The native DAW plugin (VST3/CLAP) ships alongside it in `crates/chimera-vst`.
3. **More realistic without losing what it is**: the robotic, primitive sound people like today
   stays available, and every realism change is checked by ear against it.
4. **Languages enter through one shared front-end design**, not one-off hacks. Each needs a
   native speaker's check and must leave English unchanged.
5. **The design language (the vector display) is settled**; the look within it can still evolve.
6. **Nothing copyrighted in git**; MIT; a deterministic, real-time-safe engine.
7. **Not now**: a DAW or sequencer, a light theme, MPE, multi-voice dialogue. (The fork steps past
   points 1 and 7 on purpose: it has other engines, effects and a band of voices.)

## What SIREN adds

- **Four engines for one head.** The **Airway** (the original branched FDTD simulation), a
  **Waveguide** (a Kelly–Lochbaum travelling-wave tube with a nasal branch, yielding walls,
  Bernoulli folds and jet losses: about a third of the cost), a **Formant** engine (resonators
  read off the head's own area function), and a **Neural** engine (a 14→32→32→16 SiLU MLP trained
  on 4,500+ vocal-tract postures via `bun tools/train-neural-voice.ts`, driving a hybrid
  cascade–parallel studio vocal filter with Sundberg epilaryngeal presence). Articulators,
  gestures, fold source and telemetry are shared across all four.
- **Harmony voices**: up to six extra fold sources through the same mouth (octaves, third, fifth,
  the golden interval), in any engine, at almost no cost.
- **The golden voice** (`golden`, 0 … 1): the fold pulse takes golden-section proportions (open
  for 1/φ of the cycle, opening for 1/φ of that) and two unison voices join a golden comma
  (8.33 cents) either side. It is a timbre and chorus setting, not a law of acoustics.
- **The rack**: eight effects in the engine (Punch, Nitro, Phone, Cow, Delay, Grid, Spring and a
  Golden reverb whose delays stand in golden ratio), four controls each, a limiter, and a
  recorder that saves what you hear as a WAV.
- **Russian**, by rule (`crates/chimera/src/phonetics/russian.rs`): type Cyrillic in Speak, Sing
  or Band. Stress comes from a mark (`молок+о` or `молоко́`), ё, a list of common words, then a
  guess, so mark it where it matters.
- **Band**: up to four voices at once, each its own engine and body, singing a tune in harmony
  (thirds and fifths within its key, or an octave below) or beatboxing under it.
- **Follow mic** (Play): the head follows your voice's pitch and vowel. The microphone is only
  analysed.
- **Eight creatures in the Bestiary**: alongside the songbird, Weddell seal, American alligator
  and African elephant, SIREN adds the **domestic cat** (26 Hz neural purr, `[m-i-a-u]` meow vowel
  arc, chirrup and palatal hiss), **American bullfrog** (inflatable subgular vocal sac branch),
  **Rocky Mountain elk** (true mammalian biphonia: laryngeal roar + resonance-locked palatal
  `Whistle`) and **sperm whale** (`ClickTrain` phonic lips echoing through the spermaceti sac and
  junk lens into social codas, foraging creaks and slow clangs).
- **Ableton Live VST3 & CLAP Synthesizer Plugin** (`crates/chimera-vst`): native instrument
  plugin built on `nih-plug` + `nih_plug_egui` with a custom phosphor-vector GUI, interactive
  2D XY vowel pad (`/i/–/u/–/æ/–/ɑ/`), live vocal-tract cutaway + oscilloscope, Human / Creature
  Bestiary / Vocal Beatbox MIDI modes, 16-note MIDI polyphonic stack with portamento, pitch bend,
  Mod Wheel vibrato, and studio FX rack.

Checks for these: `cargo test -p chimera --test lite_engines --test creatures_extra --test fx --test russian`,
`cargo test -p chimera-vst`, and `bun tools/e2e-voices.ts` against `bun run preview`.

## Ableton Live Setup on macOS (VST3)

### Option A — Quick Install from GitHub Releases (Apple Silicon Mac, no dev tools needed)

1. Download **`Siren-macOS-arm64-VST3.zip`** from [GitHub Releases](https://github.com/empiresfall/siren-vocal-synth/releases/latest) and unzip it.
2. Copy `Siren.vst3` into your macOS VST3 folder and clear macOS Gatekeeper quarantine:
   ```sh
   mkdir -p ~/Library/Audio/Plug-Ins/VST3
   cp -R ~/Downloads/Siren.vst3 ~/Library/Audio/Plug-Ins/VST3/
   xattr -cr ~/Library/Audio/Plug-Ins/VST3/Siren.vst3
   codesign --force --deep --sign - ~/Library/Audio/Plug-Ins/VST3/Siren.vst3
   ```
3. Open **Ableton Live → Settings (`⌘,`) → Plug-Ins**:
   - Turn **ON** `Use VST3 Plug-In System Folders`
   - Click **Rescan**
4. Load **Plug-Ins → VST3 → CHIMERA → SIREN Vocal Synth** onto a MIDI track!

### Option B — Build & Install from Source (Apple Silicon or Intel Mac)

1. Install Xcode Command Line Tools and Rust:
   ```sh
   xcode-select --install
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   source "$HOME/.cargo/env"
   ```
2. Clone the repo and run the one-step VST3 builder/installer:
   ```sh
   git clone https://github.com/empiresfall/siren-vocal-synth.git
   cd siren-vocal-synth
   ./tools/build-vst.sh --install
   ```
3. In **Ableton Live → Settings (`⌘,`) → Plug-Ins**, enable **Use VST3 Plug-In System Folders**, click **Rescan**, and drop **CHIMERA → SIREN Vocal Synth** onto any MIDI track.

## What the Web App does

- **Play**: drag the tongue, lips and jaw in a live cutaway of the head; vowel pad, encoders,
  computer keyboard and Web MIDI.
- **Speak**: English text (CMUdict with a letter-to-sound fallback), Russian Cyrillic, or IPA,
  turned into gestures and prosody, with the gestural score and a consonant chart on screen.
- **Sing**: piano roll with lyrics per note; consonants land before the beat.
- **Beatbox**: pads and a step sequencer, every drum an articulatory gesture.
- **Bestiary**: songbird (two-voice syrinx), seal, alligator, elephant, cat, bullfrog, elk and sperm whale.
- **Duet** (`?show=<id>`): two 3D heads replay a pre-rendered song.

Male, female and child bodies blend continuously. Everything on screen animates from live
engine state: pressure in the airway, airflow, the vocal folds, spectra and formants.

## Run Web App Locally

Needs Rust with the `wasm32-unknown-unknown` target, and Bun.

```sh
bun install
bun run dev        # builds the wasm, then Vite on 127.0.0.1:5317
bun run test       # wasm build, cargo test --workspace, bun test tests
```

The AudioWorklet needs a secure context, so open it on localhost or over HTTPS (for example
`tailscale serve`). Vercel builds it automatically via `vercel.json` at [siren-vocal-synth.vercel.app](https://siren-vocal-synth.vercel.app).

Offline renders and measurement:

```sh
cargo run --release -p chimera-cli -- render speech|text|song|notes|beatbox|creature|vowels …
cargo run --release -p chimera-cli -- diagnose       # nasal leakage, formants, bandwidths, pitch
cargo run --release -p chimera-cli -- compare csd    # against a real singer (docs/csd-comparison.md)
cargo run --release -p chimera-cli -- compare tsp    # against 23 real speakers (docs/speech-comparison.md)
bun tools/spectrogram.ts in.wav out.png
bun tools/musicxml-to-notes.ts score.mxl P1 --out p1.json   # a score's part → `render notes` melody
```

## Docs

- [docs/architecture.md](docs/architecture.md): the acoustic model (FDTD airway, sources,
  anatomy, gestures), and its [references and credits](docs/architecture.md#references-and-credits).
- [docs/rust-engine.md](docs/rust-engine.md): the engine's real-time and determinism contract,
  and the browser boundary.
- [docs/ui-redesign.md](docs/ui-redesign.md) and [docs/mockups/](docs/mockups/): the vector
  display look.
- [docs/csd-comparison.md](docs/csd-comparison.md): scoring the voice against the Children's
  Song Dataset.
- [docs/speech-comparison.md](docs/speech-comparison.md): scoring the speech against real
  speakers, phone by phone (the TSP speech database).
- [docs/hill-climb/](docs/hill-climb/): the rules and log for tuning the voice one measured
  step at a time.

Agents: see [AGENTS.md](AGENTS.md).

## Links & Credits

- **Live Web Synthesizer (Vercel):** [siren-vocal-synth.vercel.app](https://siren-vocal-synth.vercel.app)
- **Instagram — Vibe Coding News:** [@vibecoding_news](https://www.instagram.com/vibecoding_news/)
- **Upstream Engine:** [jamesondh/chimera-vocal-synth](https://github.com/jamesondh/chimera-vocal-synth) by Jameson Hodge (MIT)

## License

[MIT](LICENSE). Bundled third-party files keep their own licenses: the CMU Pronouncing
Dictionary ([public/data/CMUDICT-LICENSE.txt](public/data/CMUDICT-LICENSE.txt)) and the fonts
([public/fonts/](public/fonts/)).
