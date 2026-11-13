//! Acoustic, sample-rate and block-size checks for the fork's four extra creatures:
//! domestic cat, American bullfrog, Rocky Mountain elk and sperm whale.
use chimera::creatures::{CreatureVoice, creature_by_id};

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn every_extra_creature_call_sounds_cleanly_and_is_block_independent() {
    for id in ["cat", "bullfrog", "elk", "whale"] {
        let def = creature_by_id(id).expect(id);
        for (ci, call) in def.calls.iter().enumerate() {
            let dur = call.keys.last().map_or(1.0, |k| k.t) + 0.15;
            let n = (dur * 48000.0) as usize;

            let render_blocks = |block: usize| {
                let mut v = CreatureVoice::new(def, 48000.0, &[]);
                v.play_call(ci, None, 1.0);
                let mut out = vec![0.0f32; n];
                for chunk in out.chunks_mut(block) {
                    v.render(chunk);
                }
                out
            };

            let ref_out = render_blocks(128);
            assert!(ref_out.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{id}/{}: non-finite or clipped", call.id);
            let level = rms(&ref_out);
            let peak = ref_out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            println!("{id}/{} ({:.2}s): rms {level:.4}, peak {peak:.3}", call.id, dur);
            assert!(level > 0.004 && peak > 0.02, "{id}/{}: too quiet (rms {level}, peak {peak})", call.id);

            let by_77 = render_blocks(77);
            assert!(by_77 == ref_out, "{id}/{}: block size 77 differs from 128", call.id);
        }
    }
}

#[test]
fn extra_creatures_hold_across_device_sample_rates() {
    for id in ["cat", "bullfrog", "elk", "whale"] {
        let def = creature_by_id(id).expect(id);
        for fs in [16000.0, 24000.0, 44100.0, 48000.0, 96000.0] {
            let mut v = CreatureVoice::new(def, fs, &[]);
            v.set_gate(1.0);
            let mut out = vec![0.0f32; fs as usize];
            for chunk in out.chunks_mut(128) {
                v.render(chunk);
            }
            assert!(out.iter().all(|x| x.is_finite()), "{id} went NaN at {fs} Hz");
            let r = rms(&out[out.len() / 4..]);
            assert!(r > 0.002, "{id} is silent on sustained gate at {fs} Hz (rms {r})");
        }
    }
}
