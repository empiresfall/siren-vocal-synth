/**
 * Headless-Chromium check of what SIREN adds: the four engines, the harmony and golden
 * voices, the rack (and its recorder), Russian in Speak and Sing, the band, and the extra creatures.
 * Usage: bun tools/e2e-voices.ts [url] [outdir]   (against `bun run preview`)
 */
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";

const url = process.argv[2] ?? "http://127.0.0.1:5318/";
const outDir = process.argv[3] ?? "out/e2e";
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ headless: true, args: ["--autoplay-policy=no-user-gesture-required", "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"] });
const errors: string[] = [];
const page = await browser.newPage({ viewport: { width: 1500, height: 1050 }, deviceScaleFactor: 1, permissions: ["microphone"] });
page.on("console", (m) => m.type() === "error" && errors.push(`console: ${m.text()}`));
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
const log = (s: string) => console.log(s);
const fail = (s: string) => errors.push(s);

/** level of the app's own voice, and of everything heard (the analyser after the rack) */
const levels = () =>
  page.evaluate(() => {
    // @ts-expect-error test hook
    const e = window.__chimeraEngine;
    const a = e.analyserScope as AnalyserNode;
    const buf = new Float32Array(4096);
    a.getFloatTimeDomainData(buf);
    let s = 0, bad = 0;
    for (const v of buf) (s += v * v), (bad += Number.isFinite(v) ? 0 : 1);
    return { voice: (e.latest?.t?.outRms ?? -1) as number, heard: Math.sqrt(s / buf.length), bad, load: e.load as number, error: e.error as string | null };
  });

await page.goto(url, { waitUntil: "networkidle" });
await page.getByRole("radio", { name: "Play", exact: true }).click();
await page.getByRole("button", { name: /breathe in and start/i }).click();
await page.waitForTimeout(1500);

// ---- the four engines, each holding a note
const engineLoads: Record<string, number> = {};
for (const name of ["Wave", "Formant", "Neural", "Airway"]) {
  await page.getByRole("radiogroup", { name: "Engine" }).getByRole("radio", { name }).click();
  await page.waitForTimeout(1600);
  const l = await levels();
  engineLoads[name] = l.load;
  log(`engine ${name}: voice rms ${l.voice.toFixed(4)}, load ${(l.load * 100).toFixed(0)} %`);
  if (!(l.voice > 0.02) || l.bad || l.error) fail(`engine ${name} is not sounding: ${JSON.stringify(l)}`);
  await page.screenshot({ path: `${outDir}/voices-engine-${name.toLowerCase()}.png` });
}
if (!(engineLoads.Formant < engineLoads.Airway && engineLoads.Wave < engineLoads.Airway && engineLoads.Neural < engineLoads.Airway)) fail(`the light engines are not lighter: ${JSON.stringify(engineLoads)}`);

// ---- harmony adds a fifth; the golden voice reshapes the pulse (1/phi) and beats in slow chorus
const solo = (await levels()).voice;
await page.getByRole("group", { name: "Harmony voices" }).getByRole("button", { name: "5th" }).click();
await page.waitForTimeout(700);
const duo = (await levels()).voice;
await page.getByRole("group", { name: "Harmony voices" }).getByRole("button", { name: "5th" }).click();
await page.waitForTimeout(200);
const goldSlider = page.getByRole("slider", { name: "Golden" });
await goldSlider.focus();
await page.keyboard.press("End");
const goldText = (await goldSlider.getAttribute("aria-valuetext")) ?? "";
// the golden voice's three sources drift in and out of phase (±8.3 ct chorus): measure its swell
let goldMin = Infinity, goldMax = 0;
for (let i = 0; i < 14; i++) {
  await page.waitForTimeout(180);
  const v = (await levels()).voice;
  goldMin = Math.min(goldMin, v);
  goldMax = Math.max(goldMax, v);
}
log(`solo ${solo.toFixed(4)}, with a fifth ${duo.toFixed(4)}, golden ${goldMin.toFixed(4)}..${goldMax.toFixed(4)} (${goldText})`);
if (!(duo > 1.05 * solo)) fail("the harmony voice adds nothing");
if (!goldText.includes("100 %") || !(goldMax > 0.06 && goldMax > 1.12 * goldMin)) fail(`the golden voice did not engage: ${goldMin}..${goldMax} (${goldText})`);
await page.keyboard.press("Home");

// ---- every effect: on, heard and finite; and a recording of it
await page.getByRole("button", { name: "Rec", exact: true }).click();
for (const fx of ["Punch", "Nitro", "Phone", "Cow", "Delay", "Grid", "Spring", "Golden"]) {
  const chip = page.getByRole("group", { name: "Effects" }).getByRole("button", { name: fx, exact: true });
  await chip.click(); // opens it and switches it on (Punch is open already: it switches on)
  if ((await chip.getAttribute("aria-pressed")) !== "true") await chip.click();
  await page.waitForTimeout(450);
  const l = await levels();
  log(`fx ${fx}: heard ${l.heard.toFixed(4)}`);
  if (!(l.heard > 0.005) || l.bad || l.error) fail(`with ${fx} on: ${JSON.stringify(l)}`);
  if (fx === "Cow") await page.screenshot({ path: `${outDir}/voices-rack.png` });
  await chip.click();
  if ((await chip.getAttribute("aria-pressed")) !== "false") fail(`${fx} did not switch off`);
}
await page.getByRole("button", { name: "Stop", exact: true }).click();
const save = page.getByRole("link", { name: "Save WAV" });
await save.waitFor({ timeout: 5000 });
const wav = await page.evaluate(async () => {
  const a = document.querySelector("a.take") as HTMLAnchorElement;
  const b = new Uint8Array(await (await fetch(a.href)).arrayBuffer());
  const v = new DataView(b.buffer);
  let peak = 0;
  for (let o = 44; o + 1 < b.length; o += 2) peak = Math.max(peak, Math.abs(v.getInt16(o, true)));
  return { tag: String.fromCharCode(...b.slice(0, 4)), seconds: (b.length - 44) / 4 / v.getUint32(24, true), channels: v.getUint16(22, true), peak: peak / 32768 };
});
log(`recording: ${JSON.stringify(wav)}`);
if (wav.tag !== "RIFF" || wav.channels !== 2 || !(wav.seconds > 3) || !(wav.peak > 0.05)) fail(`bad recording ${JSON.stringify(wav)}`);

// ---- the head follows the (fake) microphone without errors
await page.getByRole("button", { name: "Follow mic" }).click();
try {
  await page.getByRole("button", { name: "Following mic" }).waitFor({ timeout: 6000 });
  await page.waitForTimeout(1200);
  log(`mic: following; heard ${JSON.stringify(await page.locator(".rack-heard").allTextContents())}`);
  await page.getByRole("button", { name: "Following mic" }).click();
} catch {
  fail(`the microphone follower did not start: ${JSON.stringify(await page.locator(".rack-heard").allTextContents())}`);
}

// ---- Russian in Speak
await page.getByRole("radio", { name: "Speak", exact: true }).click();
const box = page.locator(".say textarea, .say input[type=text]").first();
await box.fill("Привет, мир! Как дела?");
await page.getByRole("button", { name: /^speak$/i }).click();
await page.waitForTimeout(900);
const spoken = await levels();
const ipa = await page.locator(".say").innerText();
log(`russian speech: voice rms ${spoken.voice.toFixed(4)}; ${ipa.split("\n").find((l) => l.includes("/")) ?? ""}`);
if (!(spoken.voice > 0.005)) fail("Russian text is not spoken");
if (!/prʲ?ɪvʲ?ˈ[eɛ]t|prɪvˈ[eɛ]t/.test(ipa.replace(/\s/g, ""))) fail(`Russian transcription missing: ${ipa.slice(0, 200)}`);
await page.screenshot({ path: `${outDir}/voices-russian.png` });
await page.waitForTimeout(2500);

// ---- Russian in Sing
await page.getByRole("radio", { name: "Sing", exact: true }).click();
await page.getByLabel("Lyrics to sing").fill("Калинка, малинка мо+я");
const syls = await page.locator(".syllables .syl").allTextContents();
log(`russian lyrics: ${syls.join(" ")}`);
if (syls.join("") !== "Калинкамалинкамоя") fail(`Russian lyrics split as ${JSON.stringify(syls)}`);
// an on-screen key (the computer keyboard is typing in the lyrics box)
const key = await page.locator(".keys .key.white").nth(4).boundingBox();
await page.mouse.move(key!.x + key!.width / 2, key!.y + key!.height * 0.7);
await page.mouse.down();
await page.waitForTimeout(700);
const sung = (await levels()).voice;
await page.mouse.up();
if (!(sung > 0.02)) fail(`Russian syllable not sung (rms ${sung})`);

// ---- the band: three voices, in the light engines
await page.getByRole("radio", { name: "Band", exact: true }).click();
await page.getByRole("button", { name: "Play the band" }).click();
await page.waitForTimeout(3500);
const leadOnly = await levels();
log(`band: lead ${leadOnly.voice.toFixed(4)}, everyone ${leadOnly.heard.toFixed(4)}, load ${(leadOnly.load * 100).toFixed(0)} %`);
if (!(leadOnly.heard > 0.02) || leadOnly.bad || leadOnly.error) fail(`the band is not sounding: ${JSON.stringify(leadOnly)}`);
if (!(leadOnly.load < 0.9)) fail(`the band overloads the audio thread: ${leadOnly.load}`);
await page.screenshot({ path: `${outDir}/voices-band.png` });
await page.getByRole("button", { name: "Stop the band" }).click();
await page.waitForTimeout(900);
const quiet = await levels();
if (quiet.heard > 0.01) fail(`the band did not stop (heard ${quiet.heard})`);

// ---- the four extra creatures in Bestiary (sample peak across the call so rhythmic click codas are caught)
await page.getByRole("radio", { name: "Bestiary", exact: true }).click();
await page.waitForTimeout(300);
for (const cname of ["Domestic cat", "American bullfrog", "Rocky Mountain elk", "Sperm whale"]) {
  await page.locator(".specimen", { hasText: cname }).click();
  await page.waitForTimeout(200);
  await page.locator(".call").first().click();
  let peakRms = 0;
  let lastId: string | undefined;
  let lastErr: string | null = null;
  for (let i = 0; i < 10; i++) {
    await page.waitForTimeout(90);
    const snap = await page.evaluate(() => {
      // @ts-expect-error test hook
      const e = window.__chimeraEngine;
      const a = e?.analyserScope as AnalyserNode | undefined;
      let heard = 0;
      if (a) {
        const buf = new Float32Array(4096);
        a.getFloatTimeDomainData(buf);
        let s = 0;
        for (const v of buf) s += v * v;
        heard = Math.sqrt(s / buf.length);
      }
      return { id: e?.latestCreature?.t?.id as string | undefined, rms: Math.max(e?.latestCreature?.t?.outRms ?? 0, heard), error: (e?.error ?? null) as string | null };
    });
    lastId = snap.id ?? lastId;
    peakRms = Math.max(peakRms, snap.rms);
    if (snap.error) lastErr = snap.error;
  }
  log(`creature ${cname}: id=${lastId}, rms=${peakRms.toFixed(4)}`);
  if (!(peakRms > 0.005) || lastErr) fail(`creature ${cname} failed: ${JSON.stringify({ id: lastId, rms: peakRms, error: lastErr })}`);
}
await page.screenshot({ path: `${outDir}/voices-bestiary-8.png` });

// ---- back in Play nothing is left over
await page.getByRole("radio", { name: "Play", exact: true }).click();
await page.waitForTimeout(500);
const end = await levels();
if (end.error) fail(`engine error: ${end.error}`);

await browser.close();
console.log("errors:", errors.length ? "\n" + errors.join("\n") : "none");
if (errors.length) process.exit(1);
