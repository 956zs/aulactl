//! READ-only probe: identifies the board, prints its capabilities on stderr and dumps a full snapshot as
//! JSON on stdout. Sends only READ commands. Run with AULACTL_DEBUG=1 to see every frame.

use aula_core::hidraw;
use aula_core::Keyboard;

fn main() -> Result<(), String> {
    let devices = hidraw::find_devices();
    for d in &devices {
        eprintln!("{}  {:?}  {}", d.node.display(), d.name, d.phys);
    }
    let info = devices.first().ok_or_else(|| aula_core::Error::NotFound.to_string())?;
    let mut kb = Keyboard::open(info)?;
    let id = kb.identity().clone();
    eprintln!("identity: {}  (build {})", id.raw, id.build_date);
    eprintln!("product code {} firmware {} supported {}", id.product_code, id.firmware, kb.is_supported());
    if !kb.is_supported() {
        return Err(aula_core::Error::UnsupportedProduct { product_code: id.product_code }.into());
    }
    let caps = *kb.capabilities()?;
    for (name, has) in caps.features() {
        eprintln!("  {name:<20} {}", if has { "yes" } else { "no" });
    }
    eprintln!("win-lock blocks {:?}; 6KRO {}", caps.win_lock(), caps.six_key_rollover);
    let limits = kb.travel_limits()?;
    eprintln!("travel {}..{} x {} mm", limits.min, limits.max, limits.unit_mm());
    eprintln!("mode list {:?}", kb.read_mode_list()?);
    eprintln!("fn-fixed keys {:?}", kb.read_fn_fixed()?);
    eprintln!("switch types offered {:?}", kb.read_switch_list()?);
    let t = std::time::Instant::now();
    let snap = kb.read_snapshot()?;
    eprintln!("snapshot read in {} ms", t.elapsed().as_millis());
    println!("{}", snap.to_json()?);
    Ok(())
}
