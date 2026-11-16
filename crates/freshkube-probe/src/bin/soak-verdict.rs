//! Judges a soak run's samples: `soak-verdict <samples.csv> [warmup-seconds]`.
//! Exits 0 when no series grew steadily, 1 when one did, 2 when the file
//! can't be read or the run is too short to judge. `scripts/soak.sh` runs it.

use freshkube_probe::soak;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: soak-verdict <samples.csv> [warmup-seconds]");
        std::process::exit(2);
    };
    let warmup = args.next().and_then(|w| w.parse().ok()).unwrap_or(600);
    let samples = std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| soak::parse_csv(&text))
        .unwrap_or_else(|error| {
            eprintln!("soak: {path}: {error}");
            std::process::exit(2);
        });
    match soak::check(&samples, warmup) {
        Err(unjudged) => {
            eprintln!("soak: {unjudged}");
            std::process::exit(2);
        }
        Ok(grown) if grown.is_empty() => {
            let (kept, parts) = soak::part_medians(&samples, warmup);
            println!("soak: no steady growth in {kept} samples after the warm-up");
            for (name, [a, b, c]) in parts {
                println!("soak:   {name}: {a:.1}, {b:.1}, {c:.1} (medians of thirds)");
            }
        }
        Ok(grown) => {
            for growth in &grown {
                println!("soak: {growth}");
            }
            std::process::exit(1);
        }
    }
}
