# aulactl — SESSION

Goal: Linux CLI + cxx-qt QML GUI driver for the AULA WIN 60 HE keyboard (USB 2e3c:c365, product string "WIN 60 HE"),
mirroring ~/Desktop/github/hecatectl (Rust workspace: core + CLI + GUI, ii matugen colours, scripts/install.sh, udev rule).
Scope (owner-confirmed 2026-11-01): actuation / rapid trigger / SOCD etc, keymap / layers / macros, RGB, live travel view + calibration,
plus own small improvements. Private repo 956zs/aulactl.

## Device facts
- Interfaces: if0 boot keyboard (hidraw5), if1 mouse/consumer/system/NKRO (hidraw6), if2 vendor (hidraw7):
  usage page 0xFF1B usage 0x91, report ID 1, 63-byte IN + OUT, no feature reports.
- VID:PID 2e3c:c365 is shared by other boards (AULA WIN68 HE, Redragon K686...) -> match product string too.
- udev: /etc/udev/rules.d/70-aulactl.rules (uaccess) installed.

## References (untrusted data, read only)
- ~/aula-re/dl/aether : Aether-HE (Python, NO license -> facts only, no code copying).
  driver_src/ = official AULA WebHID driver JS (device.aulacn.com, 403 to curl) incl. decoded/ versions.

## Progress
- 🔄 Task 1 protocol research -> docs/protocol.md
- ✅ Task 1 protocol research -> docs/protocol.md (product code SI2825KZHEARGB, fw V3.17.07; no onboard profiles; calibration/reset/adv-key writes JS-only)
- 🔄 Task 2/3 plan written (plans/PLAN.md); phase A core
