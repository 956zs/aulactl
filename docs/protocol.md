# AULA WIN 60 HE — vendor HID protocol

Status: research complete, 2026-11-01. Written to be implementable without reading any other source.

Sources, in order of authority:

1. **Official AULA WebHID driver** (device.aulacn.com / hed.aulacn.com): `agreement.min.js` (all HID I/O),
   `wmIndex.min.js` (UI logic), `variable.min.js` (data classes), `config/*.json` (device list, key layouts,
   keycode table, UI strings). Two builds were read; the newer one only adds the onboard-profile commands
   (section 9.9) to the HID layer.
2. **Live probe of the owner's board** (this session, READ-only, raw frames in Appendix A).
3. **A third-party WebHID capture** of the official driver talking to another WIN 60 HE on the same firmware
   (V3.17.07, build "Apr 15 2026,11:20:35" — identical to the owner's board), made by the Aether-HE project. It is the only on-the-wire
   evidence for WRITE commands. Aether-HE's own notes were used as hints only; where they disagree with the
   official JS, the JS wins and the disagreement is listed in section 15.

Confidence tags used below:

| tag | meaning |
|---|---|
| **[DEV]** | verified on the owner's board in this session (READ commands and travel streaming only) |
| **[CAP]** | write seen on the wire in the third-party capture of the official driver (same model and firmware) |
| **[JS]** | from the official driver source only, never observed on the wire |
| **[?]** | uncertain: inferred, or the source is ambiguous |

Every command is marked **READ** (no state change) or **WRITE** (changes device state). WRITE commands that
the official driver only issues behind a confirmation dialog are marked **DESTRUCTIVE**. Commands that must
never be sent by aulactl are marked **DO NOT USE**.

---

## 1. Device identification

| property | value |
|---|---|
| USB VID:PID | `2e3c:c365` [DEV] |
| USB product string | `WIN 60 HE` (no manufacturer string, no serial) [DEV] |
| bcdDevice | `0x0200` [DEV] |
| USB speed | high speed (480 Mb/s) [DEV] |
| interfaces | if0 = boot keyboard (class 3/1/1), if1 = mouse/consumer/system/NKRO, if2 = **vendor** [DEV] |
| vendor collection | usage page `0xFF1B`, usage `0x91`, report ID `1`, one 63-byte Input report and one 63-byte Output report, no Feature reports [DEV] |

Vendor report descriptor as read from sysfs [DEV]:

```
06 1b ff  09 91  a1 01  85 01  19 00 2a ff 00  15 00 26 ff 00  75 08 95 3f  81 02
19 00 29 ff  91 02  c0
```

### 1.1 Telling WIN 60 HE apart from other `2e3c:c365` boards

`2e3c:c365` is shared. The official device list (`config/device.json`) maps it to five boards, all company
`suoai` (AULA / 狼蛛):

| name | product code (cmd `0x0D` field 4) | layout file |
|---|---|---|
| WIN 60 HE | `SI2825HEARGB` | 61 keys, this document |
| WIN 60 HE | `SI2825KZHEARGB` | identical to `SI2825HEARGB` (byte-identical layout JSON) |
| WIN 68 HE | `SI2828HEARGB` | different (68 keys) |
| WIN 68 HE | `SI2828KZHEARGB` | different |
| KP-TE153 | `SI2851UKKZHEARGB` | different |

Other vendors' boards (e.g. a Redragon K686, per the project notes) also reuse the PID. The official driver identifies the board **only**
by the product code returned by command `0x0D` (section 4.1), field index 4 of the comma-separated string.
aulactl must do the same:

1. Enumerate hidraw nodes with `HID_ID=0003:00002E3C:0000C365` whose report descriptor starts with
   usage page `0xFF1B` / usage `0x91` (on Linux the interface is input2; `HID_PHYS` ends in `/input2`).
2. Optionally pre-filter on the USB product string `WIN 60 HE`.
3. Send `0x0D` and accept only product code `SI2825HEARGB` or `SI2825KZHEARGB`. Refuse to write to anything
   else.

The owner's board reports `W669,34,KB,SI,SI2825KZHEARGB,V3.17.07` [DEV].

---

## 2. Framing

### 2.1 Transport

* Every request is an **Output report, report ID 1, 63 data bytes**. On Linux hidraw, `write()` 64 bytes:
  `0x01` followed by the 63-byte body. [DEV]
* Every reply/notification is an **Input report, report ID 1, 63 data bytes**. hidraw `read()` returns 64
  bytes with `0x01` first. [DEV]
* No checksum, no magic, no sequence number. [DEV][CAP]

**All offsets in this document are body offsets `b[n]`, report ID excluded.** On hidraw add 1.

### 2.2 Generic envelope

```
b[0]      command
b[1]      sub-selector / layer / slot / read flag (command dependent)
b[2..3]   page (chunk) index, big-endian u16
b[4]      payload length of this frame (but see cmd 0x21 and 0x14 below)
b[5..62]  payload (up to 58 bytes)
```

Unused bytes are zero in requests. In replies, bytes after `5 + b[4]` are **not** guaranteed to be zero:
the firmware leaves stale buffer contents there (seen in `0x0D` page 1 and in travel-stream frames) [DEV].
Always honour `b[4]`.

Read requests are the bare command with `b[1]` set to the read selector (usually `0x80 | x`) and every other
byte zero.

### 2.3 Request/reply discipline

* Replies carry no request ID. Send **one request at a time** and match replies by `b[0]` (and `b[1]` /
  `b[5]` where noted). Drain stale input before each request.
* Paged reads: the device answers one request with all pages back to back, ~2 ms apart [DEV]. Page count is
  fixed per table (listed per command).
* Writes: most write frames are answered by exactly one **ACK frame** (table 2.4). Lighting writes
  (`0x07`/`0x08`) and the poll-rate write produce no ACK [CAP]; the gamepad-mode write was never observed.
* The official driver does not use real timeouts: after each request it yields to the event loop at most
  10 times (a few ms) and then moves on whether or not the reply arrived. It sleeps 5 ms between per-key RGB
  pages and waits for the ACK of each macro page. Recommended for aulactl: send, then wait for the ACK or
  the expected number of pages with a **200 ms per-frame / 1 s per-table** timeout; on timeout abort the
  operation and report it. Never pipeline a second request before the first one's reply has arrived.
* Single-frame replies observed in < 1 ms; first page of tables in < 1 ms [DEV].

### 2.4 ACK frames [CAP]

| write | ACK (first 6 bytes; rest zero) |
|---|---|
| `0x09` per-key RGB page, slot s | `09 s 00 00 00 00` |
| `0x14` performance/win-lock | `14 00 00 00 00 00` |
| `0x17` music-rhythm state | `17 00 00 00 00 00` |
| `0x18` keymap page (either layer) | `18 00 00 00 00 00` |
| `0x19` macro page (any slot) | `19 00 00 00 00 00` |
| `0x21` trigger write / travel-test open / close | `21 00 00 00 01 00` [DEV for open/close] |
| `0x22` advanced-key page | `22 00 00 00 00 00` |
| `0x24` SOCD data page (`24 00 pp`) | `24 00 00 00 00 00` |
| `0x24` SOCD power (`24 03`) | **none** — the write takes effect but no ACK comes [DEV 2026-11-01]; read `24 02` instead |
| `0x25` switch-type page | `25 01 00 00 00 00` |
| `0x26` dead-band page | `26 01 00 00 00 00` |

### 2.5 Key index

Every per-key structure uses the **key index** `idx = row * 22 + col` (`0 ≤ idx < 132`, 6 rows × 22 columns
of a virtual matrix; the WIN 60 HE populates 61 of them, table in section 14).

* **Index tables** (keymap, per-key RGB, switch type, dead band) store entry `idx` at `idx * entry_size`.
* **Key-select masks** (trigger write, travel-test open, calibration progress, Fn-state) are 22 bytes,
  transposed: `mask[idx % 22] |= 1 << (idx / 22)`. The all-keys mask of this board is
  `3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e 00 00 00 00 00 00 00` (61 bits) [DEV][CAP].
* **Row/column pairs** (per-key trigger read, travel stream, RS advanced key) are `row = idx / 22`,
  `col = idx % 22`.

### 2.6 Command summary

| cmd | name | READ | WRITE |
|---|---|---|---|
| `0x01` | info / capabilities / heartbeat | `01` | — |
| `0x02` | side-light (ambient) mode list | `02` | — |
| `0x07` | keyboard lighting effect | `07 01` | `07 00` |
| `0x08` | side-light effect (no LEDs on this board) | `08 01` | `08 00` (do not use) |
| `0x09` | per-key RGB (two custom slots) | `09 80` / `09 81`, `09 20` | `09 00` / `09 01` |
| `0x0A` | keyboard lighting mode list | `0A` | — |
| `0x0D` | firmware / product string | `0D` | — |
| `0x0E` | Fn-layer fixed-key mask | `0E` | — |
| `0x14` | performance (win-lock flags, 6KRO), reset, gamepad, sleep | — | `14 …` |
| `0x17` | music-rhythm state | — | `17 00` |
| `0x18` | keymap (base / Fn layer) | `18 80` / `18 82` | `18 00` / `18 02` |
| `0x19` | macros (10 slots) | `19 80..89` | `19 00..09` |
| `0x21` | trigger family (sub-command in `b[5]`) | subs 04, 05, 09, (11) | modes 0/12/13, subs 02, 03, 06, 08, 09, 0F, 10, (07, 12) |
| `0x22` | advanced keys (40 slots) | `22 80` | `22 00..27` |
| `0x24` | SOCD ("PRCS") | `24 01`, `24 02` | `24 00`, `24 03` |
| `0x25` | switch type per key | `25 00`, `25 02` | `25 01` |
| `0x26` | dead band per key | `26 00` | `26 01` |

No other command byte is used by the official driver. **Do not send any other command byte** — behaviour is
unknown. There is no firmware-update or bootloader command over this interface: the official "firmware
update" button only downloads a Windows updater archive (`GD285CKB_W669KBSI_SI2825KZHEARGB_V31707.zip` etc.)
and sends nothing over HID [JS].

---

## 3. Connect sequence

The official driver's mount sequence (order matters only in that each step completes before the next) [CAP]:

```
0D            product / firmware string (2 frames)
01            capabilities (also starts the 1 s heartbeat)
0E            Fn-layer fixed-key mask
19 80..19 89  10 macro slots
22 80         advanced keys (40 pages)
24 02, 24 01  SOCD power, SOCD entries          (only if isPrcs)
18 80, 18 82  keymap base, keymap Fn
0A, 02        lighting mode lists
07 01, 08 01  current effect (keyboard, side)
09 20         active custom slot
09 80, 09 81  per-key RGB slot 0 and 1         (only if isCustomLight)
21 sub 04     travel limits and unit
21 sub 05 ×61 per-key trigger config
21 sub 09     poll rate                        (only if isPollRate)
25 00, 25 02  switch list, per-key switch       (only if isMoreSwitch)
26 00         dead band                        (only if isDeadBand)
21 sub 11     onboard profile index (newer driver build only; WIN 60 HE does not answer)
```

**Heartbeat:** while connected the driver sends `01` every 1000 ms and reads the reply (it uses it to detect
wireless mode changes on other boards). No evidence that the WIN 60 HE needs it; reads and the travel stream
worked without it [DEV]. aulactl may send it while a GUI session is open; it is harmless (READ).

---

## 4. Identification and capabilities

### 4.1 `0x0D` — product / firmware string — READ [DEV]

Request: `0D 00 …`.

Two reply frames:

| `b[3]` (page) | `b[4]` | payload |
|---|---|---|
| 0 | `0x32` | ASCII, NUL-padded: `W669,34,KB,SI,<product code>,V<major>.<minor>.<patch>` |
| 1 | `0x32` | ASCII build date, e.g. `Apr 15 2026,11:20:35`, then NUL, then stale bytes |

Parse each payload up to the first NUL. Split page 0 on `,`: field 4 = product code (section 1.1), field 5 =
firmware version (`V3.17.07` → `3.17.07`; the official driver removes `V` and control characters and turns
`_` into `.`). Fields 0–3 (`W669`, `34`, `KB`, `SI`) are not interpreted by the driver. If the string
contains `dongle`, the board is behind a 2.4 GHz dongle (not applicable to this wired board).

Note: the official parser takes `subarray(5, b[4])` (end index = length byte, a bug that drops the last five
characters of a 50-byte string). Use `b[5 .. 5 + b[4]]`.

### 4.2 `0x01` — info / capabilities / heartbeat — READ [DEV]

Request: `01 00 …`. Reply `b[4] = 0x11` (17). Owner's board:

```
01 00 00 00 11 00 00 00 00 00 00 00 00 00 01 00 00 7f 00 01 01 00 00 …
```

| offset | meaning | owner's board |
|---|---|---|
| `b[1]` | connection-mode token (driver reloads if it changes between heartbeats) | 0 |
| `b[8]` | 1 = charging (wireless boards) | 0 |
| `b[9]` | battery % (wireless boards) | 0 |
| `b[10]` | `triMode`: 0 = USB, 1 = 2.4 GHz | 0 (USB) |
| `b[14]` | `isPrcs` = SOCD supported (`== 1`) | 1 |
| `b[17]` | feature flags 1 | `0x7f` |
| `b[18]` | feature flags 2 | `0x00` |
| `b[19]` | not used by the driver [?] | 1 |
| `b[20]` | current win-lock flag set (same bit layout as the `0x14` write, section 11.1) | `0x01` (Win key disabled while win-lock is on) |
| `b[21]` | `isMusicRhythmOn` (`== 1`) | 0 |
| `b[22]` | bit 0: 6-key rollover mode active (else NKRO) | 0 |

`b[17]` bits: 0 `isAnyKeyCalibration`, 1 `isCustomLight`, 2 `isMoreSwitch` (switch-type table), 3
`isPollRate`, 4 `isDeadBand`, 5 `isMusicRhythm`, 7 `isAllAnd6keySwitch` (NKRO/6KRO selector).
`b[18]` bits: 0 `isHighPrecision` (trigger divisor 1000 instead of 100), 2 `isGamePad`, 3 `isRS`, 4 `isRKRT`.

WIN 60 HE result: SOCD, any-key calibration, per-key RGB, switch types, poll rate, dead band, music rhythm =
**yes**; NKRO/6KRO selector, high precision, gamepad, RS, RKRT = **no**. aulactl should read this frame on
connect and gate features on it rather than hard-coding.

### 4.3 Unsolicited / special Input frames

| frame | meaning | required host behaviour |
|---|---|---|
| `21 … b[5]=01` | travel-stream sample (section 10) | only while the travel test is open |
| `21 … b[5]=08` or `0F` | calibration progress (section 10.3) | only while calibrating |
| `21 … b[5]=11` | older driver build: "trigger config changed on device" → it turns RT off in the UI and re-reads all `21 sub 05`; newer build: reply to the profile-index query [JS][?] | re-read trigger config |
| `14 … b[5]=01` | the board reports a reset (likely a key-combo factory reset) [JS][?] | re-read everything. **Never answer by sending a reset** (the official driver's handler programmatically clicks its own "reset all" button, which sends one). |

---

## 5. Lighting

### 5.1 `0x0A` / `0x02` — mode lists — READ [DEV]

Request `0A 00 …` (keyboard) or `02 00 …` (side/ambient). Reply payload (`b[5..5+b[4]]`):
`mode ids…, 0xA5, 0x5A, maxSpeed, maxBrightness`.

Owner's board:

```
0A: b[4]=0x12  02 01 03 09 04 00 0e 0f 06 07 0b 0c 10 08 a5 5a 04 04
02: b[4]=0x04  a5 5a 04 04          → no side LEDs on WIN 60 HE
```

The official UI lists the advertised ids, then appends `100` (music rhythm, if `isMusicRhythm`) and `10`
(custom per-key, always). Brightness range `0..maxBrightness` (4), speed range `0..maxSpeed` (4).

### 5.2 Mode ids and their controls [JS][DEV for the list]

| id | name (official EN) | brightness | speed | fg/bg colour + fullColor | direction values offered |
|---|---|---|---|---|---|
| 0 | Static | yes | — | fg only (no bg, no fullColor) | — |
| 1 | Breath | yes | yes | yes | — |
| 2 | Wave | yes | yes | yes | 0 right, 1 left, 2 up, 3 down, 4 spread, 5 gather |
| 3 | Neon | yes | yes | yes | — |
| 4 | Radar | yes | yes | yes | 0 clockwise, 1 anticlockwise |
| 6 | Reactive | yes | yes | yes | — |
| 7 | Aurora | yes | yes | yes | 4 spread (default), 5 gather |
| 8 | Ripple | yes | yes | yes | — |
| 9 | Twinkle | yes | yes | yes | — |
| 10 | Custom (per-key, section 5.5) | yes | — | — | — |
| 11 | Cross | yes | yes | yes | — |
| 12 | Speed Respond | yes | — | yes | 2 up (default), 3 down |
| 14 | Auto Ripple | yes | yes | yes | — |
| 15 | Striation | yes | yes | yes | 0 right, 1 left |
| 16 | Fireworks | yes | yes | yes | — |
| 100 | Musical Rhythm (host plugin, section 5.6) | — | — | — | — |

Ids 5 and 13 are unused. When a mode does not offer direction, the UI sends direction 0.
Side-light ids (`sideLightMode*`: 0 static, 1 breath, 2 wave, 3 collision, 4 neon, 5 close) are irrelevant
here.

### 5.3 `0x07 01` — read current effect — READ [DEV]

Request `07 01 00 …`. Reply `b[4] = 0x0C`, payload:

| payload byte | body | field |
|---|---|---|
| 0 | `b[5]` | mode |
| 1 | `b[6]` | brightness |
| 2 | `b[7]` | speed |
| 3..5 | `b[8..10]` | foreground R, G, B |
| 6..8 | `b[11..13]` | background R, G, B |
| 9 | `b[14]` | direction (the driver maps values ≥ 6 to 0) |
| 10 | `b[15]` | fullColor (1 = rainbow/full-colour, 0 = use fg/bg) |
| 11 | `b[16]` | power: **0 = on, 1 = off** |

Owner's board: `00 01 02 ff 00 ff 00 00 00 00 00 00` → Static, brightness 1, speed 2, fg magenta.
`08 01` returns the same structure for the (absent) side light.

### 5.4 `0x07 00` — set effect — WRITE [CAP]

```
b[0]=07 b[1]=00 b[2]=00 b[3]=00 b[4]=0E
b[5]=mode b[6]=brightness b[7]=speed
b[8..10]=fg RGB  b[11..13]=bg RGB  b[14]=direction  b[15]=fullColor  b[16]=power (0 on, 1 off)
```

Example (wave, bri 4, speed 4, red fg): `07 00 00 00 0e 02 04 04 ff 00 00 00 00 00 00 00 00`.
No ACK. Persistent. The official driver sends `17 … 02 00 00` (music off) before every non-music light
write and `17 … 02 01 01` before selecting mode 100 (section 5.6); aulactl should do the same.

### 5.5 `0x09` — per-key RGB (Custom mode, two slots)

Table: 132 × RGB = **396 bytes**, entry `idx` at `idx*3` (R, G, B). Non-key slots are zero.
Paged in **54-byte** pages: pages 0..6 carry 54 bytes (`b[4]=0x36`), page 7 carries 18 (`b[4]=0x12`).

* **READ** slot 0: `09 80`, slot 1: `09 81` → 8 reply pages `09 8s pp pp len data…` [DEV].
* **WRITE** slot s (0 or 1): 8 frames `09 s 00 pp len data…`, one ACK each (`09 s 00 00 00`); the driver
  sleeps 5 ms after each page [CAP]. Persistent.
* `09 20` (READ, sent once at mount) → reply `09 s 00 00 00`; the driver uses `b[1]` to tick the
  Custom1/Custom2 radio, i.e. `b[1]` = active custom slot [JS]. Owner's board answers `09 00 …` [DEV].
  Which slot mode 10 displays is selected by writing that slot's table (the UI rewrites the selected slot
  when the radio changes) [JS][?].

Owner's board: slot 0 has red on idx 46, 68, 69, 70 (W A S D); slot 1 has blue on 34 keys [DEV].

**Writing a slot switches the effect to Custom** [DEV 2026-11-01]: after `09 00` / `09 01` pages (no `07 00`
sent) `07 01` reads mode 10 (Custom), brightness 4, speed 3. aulactl reads the effect (`07 01`) before a
custom-slot write and, when it differs afterwards, writes it back (`07 00`) and verifies it; snapshot apply
writes the custom slots before the effect so the effect ends up as requested.

Host-driven animation: rewriting slot 0 at up to ~60 frames/s while in mode 10 works (Aether-HE does it)
[?]; it wears nothing observable but each frame is 8 HID writes.

### 5.6 `0x17` — music-rhythm state — WRITE [CAP]

`17 00 00 00 02 v v` with `v = 1` (enter) or `0` (leave). ACK `17 00 …`. Mode 100 is driven by a separate
vendor desktop plugin that is not part of the web driver; its audio/data protocol is unknown. aulactl should
not offer mode 100 and should always send the "leave" frame before a lighting write.

---

## 6. Keymap

### 6.1 Layout

Two layers: **base** (selector 0) and **Fn** (selector 2). Each is 132 entries × 4 bytes = **528 bytes**,
entry `idx` at `idx*4`. Paged in **56-byte** pages: pages 0..8 × 56 (`b[4]=0x38`), page 9 × 24 (`b[4]=0x18`).

* **READ** base `18 80`, Fn `18 82` → 10 pages `18 8x 00 pp len data…` [DEV].
* **WRITE** base `18 00 00 pp len data…`, Fn `18 02 00 pp len data…`, pages 0..9, one ACK
  (`18 00 00 00 00`) each [CAP]. Persistent. The official driver always rewrites **both full layers** (and
  then macros and advanced keys, sections 7 and 8) for any single change.

**All-zero entries mean "firmware default".** A board that was never written reads back all zeros except the
Fn key, which reads `01 00 00 00` [DEV]. After the official driver has written a layer, it reads back the
full explicit table (`00 29 00 00` for Esc … `01 fa 00 00` for Fn) [CAP]. In the Fn layer, zero entries keep
the firmware Fn function of that key.

To write the base layer from scratch, emit for every physical key its default entry (section 14: `00 <hid>
00 00`, Fn = `01 fa 00 00`), override the remapped ones, and leave non-key slots zero. Never write a table
built from a zero read-back without filling defaults (it would be equivalent, but the official driver never
does that and it was not tested).

### 6.2 Fn-layer fixed keys — `0x0E` — READ [DEV]

Request `0E 00 …`. Reply `b[4] = 0x16`, payload = 22-byte transposed key mask (section 2.5). Keys in the mask
have a firmware Fn function that **cannot be remapped in the Fn layer** (the UI shows "Not Remappable" and
skips them). Owner's board:

```
02 22 0e 0a 02 02 22 12 1e 3e 0e 2e 3e 20 20 20 00 00 00 00 00 00
→ idx 22-34, 46, 52-56, 68, 69, 74-78, 95-97, 100, 111, 116, 119, 121, 122 (+ 123-125, which are not physical keys)
```

### 6.3 Entry encoding (4 bytes `[t, a, b, c]`) [JS][CAP]

| `t` | function | `a` | `b` | `c` | evidence |
|---|---|---|---|---|---|
| `0x00` | keyboard key | HID usage (page 7) | 0 | 0 | [CAP] |
| `0x01` | Fn (layer key) | `0xFA` | 0 | 0 | [CAP] |
| `0x02` | lighting: cycle all modes | `0x02` | 0 | 0 | [JS] |
| `0x03` | brightness | 0 = up, 1 = down | 0 | 0 | [JS] |
| `0x04` | effect speed | 0 = up, 1 = down | 0 | 0 | [JS] |
| `0x05` | cycle direction | `0x02` | 0 | 0 | [JS] |
| `0x06` | colour | 0 = full-colour toggle, 1 = cycle fg colour, 2 = cycle bg colour | 0 | 0 | [JS] |
| `0x07` | media / system (vendor index, **not** consumer usages) | table 6.4 | 0 | 0 | [JS] |
| `0x10` | macro | the physical key's own default HID usage | macro slot 0..9 | 0 | [CAP] |
| `0x11` | lock | 0 = lock/unlock keyboard, 2 = lock/unlock Win key, 4 = turn off all lights | 0 | 0 | [JS] |
| `0x12` | mouse | 0 = button, 1 = wheel | button bitmask (1 L, 2 R, 4 M, 8 forward, 0x10 back) or wheel delta (`0x01` up, `0xFF` down) | 0 | [JS] |
| `0x13` | key combination (up to 3 usages; UI requires ≥ 1 modifier and ≥ 1 normal key) | usage 1 | usage 2 | usage 3 or 0 | [JS] |
| `0x17` | advanced key (section 8) | advanced slot 0..39 | same slot | 0 | [JS] |
| `0x18`, `0x19` | gamepad buttons (gamepad boards only) | — | — | — | not supported here |

Note on `0x12`: the official keycode table stores the button/wheel byte as "hidCode" and the 0/1 selector as
"code3" and **swaps them on the wire**: `[0x12, code3, hidCode, code4]`. The table above is the wire order.

### 6.4 Keycode table (official `config/keys.json`) [JS]

Keyboard keys, `t = 0x00`, `a` =:

| key | `a` | key | `a` | key | `a` | key | `a` |
|---|---|---|---|---|---|---|---|
| A–Z | `04`–`1D` | 1–9, 0 | `1E`–`27` | Enter | `28` | Esc | `29` |
| Backspace | `2A` | Tab | `2B` | Space | `2C` | `-` | `2D` |
| `=` | `2E` | `[` | `2F` | `]` | `30` | `\` | `31` |
| `;` | `33` | `'` | `34` | `` ` `` | `35` | `,` | `36` |
| `.` | `37` | `/` | `38` | CapsLock | `39` | F1–F12 | `3A`–`45` |
| PrintScreen | `46` | ScrollLock | `47` | Pause | `48` | Insert | `49` |
| Home | `4A` | PageUp | `4B` | Delete | `4C` | End | `4D` |
| PageDown | `4E` | Right | `4F` | Left | `50` | Down | `51` |
| Up | `52` | NumLock | `53` | KP `/` | `54` | KP `*` | `55` |
| KP `-` | `56` | KP `+` | `57` | KP Enter | `58` | KP 1–9 | `59`–`61` |
| KP 0 | `62` | KP `.` | `63` | ISO `<>` (IntlBackslash) | `64` | Menu | `65` |
| F13–F24 | `68`–`73` | IntlRo | `87` | Kana | `88` | IntlYen | `89` |
| Convert | `8A` | NonConvert | `8B` | L-Ctrl | `E0` | L-Shift | `E1` |
| L-Alt | `E2` | L-Win | `E3` | R-Ctrl | `E4` | R-Shift | `E5` |
| R-Alt | `E6` | R-Win | `E7` | | | | |

Media/system, `t = 0x07`, `a` =:

| function | `a` | function | `a` |
|---|---|---|---|
| Volume up | `10` | Mail | `20` |
| Volume down | `11` | Browser search | `21` |
| Mute | `12` | Browser favourites | `22` |
| Play/Pause | `13` | Browser home | `23` |
| Next track | `14` | Browser forward | `24` |
| Previous track | `15` | Browser back | `25` |
| Stop | `16` | Browser stop | `26` |
| Media player | `30` | Browser refresh | `27` |
| Calculator | `32` | My computer | `34` |

Predefined combinations (`t = 0x13`): Show desktop `13 E3 07 00` (Win+D), Close window `13 E2 3D 00`
(Alt+F4), Lock PC `13 E3 0F 00` (Win+L), Undo `13 E0 1D 00`, Copy `13 E0 06 00`, Paste `13 E0 19 00`,
Cut `13 E0 1B 00`, Save `13 E0 16 00`, New `13 E0 11 00`, Print `13 E0 13 00`.

Mouse (`t = 0x12`, wire order): Left `12 00 01 00`, Right `12 00 02 00`, Middle `12 00 04 00`,
Forward `12 00 08 00`, Back `12 00 10 00`, Wheel up `12 01 01 00`, Wheel down `12 01 FF 00`.

---

## 7. Macros — `0x19`

10 slots × **256 bytes**. Paged in **58-byte** pages: pages 0..3 × 58 (`b[4]=0x3A`), page 4 × 24
(`b[4]=0x18`).

* **READ** slot n: `19 (0x80|n)` → 5 pages `19 8n 00 pp len data…` [DEV].
* **WRITE** slot n: 5 frames `19 n 00 pp len data…`; the driver waits for each page's ACK (`19 00 00 00 00`)
  before the next [CAP]. The official driver writes **all 10 slots** on every apply (unused slots all zero).
* **Single-slot writes are dropped** [DEV 2026-11-01]: writing one slot (5 pages, each ACKed `19 00`) often
  leaves the slot unchanged on read-back (10/10 failures with single slots in random order). Writing all 10
  slots in order 0..9 (unchanged slots with their current image) stored every target slot (8/8 runs). aulactl
  always writes the whole 0..9 sequence and reads all 10 slots back.

### 7.1 Slot image [JS][CAP]

```
[0]    macro id (the UI's macro number); bit 7 set = "same macro already stored in an earlier slot"
[1]    play mode: 0 = once, 1 = repeat N times, 2 = press to start / press again to stop, 3 = repeat while held
[2..3] event-block length in bytes, big-endian (= 4 × events; the driver only writes [3])
[4..5] repeat count N, big-endian (used by mode 1; the capture shows 1 for "once")
[6..7] 0
[8..]  events, 4 bytes each, at 8 + 4*i
```

Event: `[code, (state << 4) | type, delay_hi, delay_lo]`

* `type` 0 = keyboard, `code` = HID usage; `type` 1 = mouse button, `code` = button bit (1 L, 2 R, 4 M —
  the write path uses these; the official *read* path maps 0/1/2 → L/M/R, an inconsistency in the vendor
  code [?]).
* `state` 1 = press, 0 = release (so byte 1 is `0x10` / `0x00` for keys, `0x11` / `0x01` for mouse).
* `delay` (ms, BE u16) is the wait **after** this event before the next one; the last event's delay is 0.
  (The UI stores "delay before step"; the driver shifts by one in both directions: each event gets the next
  step's delay, the last event 0.) aulactl always writes 0 for the last event.

Limits: 62 events per slot (`(256-8)/4`); the UI caps a macro at 20 key groups. Delay 0..65535 ms. Count
0..65535.

Empty slot: never-written slots read all `0xFF` [CAP]; slots written empty by the driver read all `0x00`
[DEV — all 10 slots on the owner's board]. Treat both (or length 0) as empty.

Captured example, slot 0, "A B C", play once [CAP]:

```
19 00 00 00 3a | 00 00 00 18 00 01 00 00 | 04 10 00 7e  04 00 00 ca  05 10 00 7a  05 00 00 cb  06 10 00 7a  06 00 00 00 | 00…
```

### 7.2 Binding

A macro runs when a key's keymap entry is `[0x10, <that key's default HID usage>, <slot>, 0]` (either layer)
[CAP]. Slot allocation in the official driver: one slot per bound key, in binding order; a second key bound
to the same macro gets its own slot with bit 7 of `[0]` set. aulactl may use a simpler scheme (slot = macro)
as long as each bound slot holds a valid image.

---

## 8. Advanced keys — `0x22` (DKS, MT, TGL; RS/RKRT not supported here)

40 slots, one page per slot.

* **READ** `22 80` → 40 frames `22 80 ss ss 28 data…` (page index = slot 0..39, 40-byte payload) [DEV].
  Owner's board: all zero.
* **WRITE** slot s: `22 s 00 00 32 data…` (slot in `b[1]`, `b[4] = 0x32` = 50) for every slot 0..39, one ACK
  (`22 00 00 00 00`) each [CAP framing]. Unused slots: all-zero payload. Payload decoding below is [JS] only
  (no advanced key was ever configured on the wire).

Payload (body offsets, identical for read and write):

| `b[5]` type | fields |
|---|---|
| 0 | empty slot |
| 1 **DKS** | `b[6..9]` = route1..route4 in **0.1 mm**; then for key j = 0..3: `b[10+8j..13+8j]` = keymap entry (section 6.3) of the j-th output key, `b[14+8j..17+8j]` = its action in the four phases |
| 2 **MT** (mod-tap) | `b[6..9]` = **hold** entry, `b[10..13]` = **tap** entry, `b[14]` = hold threshold in **10 ms** units (UI 15..100 = 150..1000 ms, default 15) |
| 3 **TGL** (toggle) | `b[6..9]` = entry to toggle (tap = latch down, tap again = release) |
| 4 RS | `b[6..9]` key 1 entry, `b[10]` row, `b[11]` col, `b[12..15]` key 2 entry, `b[16]` row, `b[17]` col (only if `isRS`) |
| 5 RKRT | `b[6..9]` key 1 entry, `b[12..15]` key 2 entry (only if `isRKRT`) |

The advanced function is attached to a physical key by that key's keymap entry `[0x17, slot, slot, 0]`.

DKS details [JS]:

* Defaults: route1 = 15 (1.5 mm, first actuation going down), route2 = route3 = `max_travel_mm*10 − 4`
  (3.0 mm: bottom-out point going down / release-from-bottom going up), route4 = route1 (1.5 mm, final release
  going up). The UI only lets the user move route1 (range 1..36); it writes the same value to route4.
* Phase bytes per output key, in order: 0 = passing route1 downwards, 1 = reaching route2 (bottom), 2 =
  leaving route3 upwards, 3 = passing route4 upwards. Values: 0 = nothing, 1 = tap (press and release at this
  point), 2 = press here and keep holding into following phases, 3 = still held (continuation of a hold that
  began in the previous phase). The editor derives 2/3 from bar widths; exact firmware semantics of 2 vs 3 are
  [?].

Max 40 advanced keys (UI limit).

---

## 9. Actuation, rapid trigger, dead band, switch type, poll rate — `0x21`, `0x25`, `0x26`

### 9.1 `0x21` framing

Cmd `0x21` puts its **sub-command in `b[5]`** and `b[4]` is a constant (`0x18` for most subs, not a
length). Data extends past `5 + b[4]`. Replies use the normal meaning of `b[4]` (payload length).

### 9.2 Travel limits and unit — `21 … 18 04` — READ [DEV]

Request `21 00 00 00 18 04 00 …`. Reply `21 00 00 00 06 04 54 01 01 01 08`:

| field | formula | owner's board |
|---|---|---|
| max travel (raw) | `b[9] << 8 | b[6]` | 340 |
| unit numerator | `b[7]` (0 means 10) | 1 |
| unit | numerator / (`isHighPrecision` ? 1000 : 100) mm | **0.01 mm** |
| min travel (raw) | `b[10]`; if 0: 2 when unit = 0.04 mm, 1 when unit = 0.08 or 0.1 mm | 8 |
| `b[8]` | unused by the driver [?] | 1 |

So travel values are in 0.01 mm, range **8..340 (0.08–3.40 mm)**.

### 9.3 Per-key trigger config — `21 … 18 05 row col` — READ [DEV]

Request `21 00 00 00 18 05 row col`. One request per key (the driver loops over all 61). Reply
`b[4] = 0x0C`:

| body | field |
|---|---|
| `b[5]` | `0x05` (echo) |
| `b[6]` | mode (0, 12, 13) |
| `b[7]`, `b[11]` | actuation travel low, high → `travel = b[11] << 8 | b[7]` |
| `b[8]`, `b[12]` | second copy of travel (low, high) [?] |
| `b[9]`, `b[13]` | RT press sensitivity (interval1) low, high |
| `b[10]`, `b[14]` | RT release sensitivity (interval2) low, high |
| `b[15]`, `b[16]` | row, col (echo) |

Owner's board, every key: `0c 64 64 32 32 00 00 00 00` → RT single-sensitivity, actuation 1.00 mm,
sensitivity 0.50 mm.

### 9.4 Trigger write — mode in `b[5]` — WRITE [CAP]

```
b[0]=21 b[1..3]=0 b[4]=18
b[5]      mode: 0 = fixed actuation point, 12 (0x0C) = rapid trigger, 13 (0x0D) = rapid trigger with separate press/release sensitivity
b[6..27]  22-byte key-select mask (section 2.5)
b[28]     travel low      b[32]  travel high
b[29]     travel low      b[33]  travel high        (same value again)
b[30]     interval1 low   b[34]  interval1 high     (RT press sensitivity)
b[31]     interval2 low   b[35]  interval2 high     (RT release sensitivity)
```

ACK `21 00 00 00 01 00`. Persistent. Every selected key gets the same values; the official driver groups keys
with identical settings and sends one frame per group.

Rules from the driver:

* travel ∈ [min, max] from 9.2, but also capped by the switch type's travel (9.6): 3.4 mm for all switch types
  this board offers.
* intervals ∈ [1, ceil(max/2)] = **1..170 (0.01–1.70 mm)**, UI default 50 (0.50 mm).
* mode 12: interval2 := interval1. Mode 13: independent.
* mode 0 still carries the key's stored intervals (the driver never sends zero intervals; a factory board
  stores 1/1) [CAP].

Example, all keys 1.00 mm fixed: `21 00 00 00 18 00 3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e 00 00 00 00 00
00 00 64 64 01 01 00 00 00 00`.

### 9.5 Dead band — `0x26`

Table: 132 × `[top, bottom]` = **264 bytes**, unit = trigger unit (0.01 mm), entry `idx` at `idx*2`.
Paged in **58-byte** pages: pages 0..3 × 58 (`b[4]=0x3A`), page 4 × 32 (`b[4]=0x20`).

* **READ** `26 00` → 5 pages `26 00 00 pp len data…` [DEV]. (The driver also calls this exact frame
  `resetDeadBand`; it is only a read — values persist across it [CAP].)
* **WRITE** `26 01 00 pp len data…`, pages 0..4, ACK `26 01 00 00 00` [CAP]. Persistent.

Range per value 0..100 (0–1.00 mm); UI default 2 (0.02 mm), UI recommendation 0.10 mm. A never-written
board reads all zero (owner's board, and the capture's first mount) [DEV][CAP].

### 9.6 Switch type — `0x25`

* **READ list** `25 00` → `25 00 00 00 len  n_hi n_lo id…` [DEV]. Owner's board: `00 04 01 02 03 05` → 4
  switch types: 1, 2, 3, 5.
* **READ table** `25 02` → 3 pages, 132 bytes, one switch id per `idx` (pages 58 + 58 + 16) [DEV]. Owner's
  board: all `01`.
* **WRITE table** `25 01 00 pp len data…`, same paging, ACK `25 01 00 00 00` [CAP]. Persistent. After
  changing the type of a key the official UI tells the user to recalibrate it.

Known switch ids (official `switchDefault`; name, travel cap): 1 HM1 3.4 mm, 2 HH1 3.4, 3 CY1 3.4, 4 TW1 3.4,
5 TC1 (TTC Kom) 3.4, 6 SL1 3.4, 7 FZ1 3.9, 8 GJP (Gateron Jade Pro) 3.4, 9 HMZ 3.4, 10 GW1 (Gateron K2-20
white) 3.9, 11 VA1 (VARO Very Peri) 3.4, 12 OTK 3.4, 13 CN1 3.4, 14 RY1 3.4, 15 WCG 3.4, 16 JY1 3.6, 17 CS1
3.4, 18 VA2 (Contactless Very Peri) 3.4, 19 CD 3.4, 21 YH1 (Jade King) 3.4, 22 EM1 (electro-magnetic) 3.5.
Only offer the ids the board lists.

### 9.7 Poll rate — `21 … sub 09`

* **READ** `21 00 00 00 01 09` → `21 00 00 00 02 09 r` with r ∈ {1, 2, 4, 8} = 1/2/4/8 kHz [DEV]. Owner's
  board: 8 (8 kHz).
* **WRITE** `21 00 00 00 00 09 01 r` [CAP]. No ACK. **The keyboard restarts (USB re-enumerates)**; the
  official UI asks for confirmation first. aulactl must close the device, wait for re-enumeration and
  reconnect.

### 9.8 Reset all triggers — `21 … 18 06` — WRITE, DESTRUCTIVE [JS]

`21 00 00 00 18 06`. Resets every key's actuation/RT config to factory. Behind a confirmation dialog in the
official UI; afterwards the driver re-reads 9.2 and 9.3. Never observed on the wire.

### 9.9 Onboard profile — `21 … sub 11 / 12` — DO NOT USE on WIN 60 HE

Newer driver builds send `21 00 00 00 00 11` (query current onboard preset) and, if the board answers, switch
presets with `21 00 00 00 00 12 n`. The WIN 60 HE does **not** answer the query (capture and owner's board)
[DEV][CAP], so it has no onboard profiles. The official driver keeps 5 host-side profiles (default + 1..4)
in browser storage and switching one rewrites every table. aulactl should implement profiles host-side the
same way and never send sub `0x12`.

---

## 10. Live key travel and calibration

### 10.1 Travel test (live depth streaming) — open/close are WRITE-to-RAM only [DEV]

* **Open**: `21 00 00 00 18 02 <22-byte key mask>` (official driver: all keys). ACK `21 00 00 00 01 00`.
* **Close**: `21 00 00 00 18 03`. ACK `21 00 00 00 01 00`.

While open the board emits a frame **only when a selected key moves** (no traffic while idle), every
~1.5–5 ms per moving key:

```
b[0]=21 b[1..3]=0 b[4]=05 b[5]=01 b[6]=row b[7]=col b[8]=depth_lo b[9]=depth_hi
```

`idx = row*22 + col`; `depth = b[9] << 8 | b[8]` in trigger units (0.01 mm), 0..max travel (340 = bottom
out). When the key returns to rest the board sends one or more short frames `b[4]=03 b[5]=01 row col` (no
depth; `b[8..]` contains stale bytes, e.g. `08 00 ff ff …`) → treat as depth 0. Sometimes a `b[4]=05` frame
with depth 0 precedes them.

Observed sequence (owner pressing L-Ctrl, idx 110):
`110 → 169 → 212 → 249 → 277 → 301 → 326 → 340 … 339 → 299 → … → 26 → [len 3 frame]`.

Whether normal key reports continue while the test is open was not checked [?]. **Always close it** (also on error/exit paths); the official
driver closes it whenever the user leaves the Travel Test tab.

### 10.2 Calibration — WRITE (rewrites stored calibration) [JS]

WIN 60 HE has `isAnyKeyCalibration = 1`, so the official driver uses the "any key" variant:

| step | frame |
|---|---|
| start | `21 00 00 00 18 0F 00` |
| user presses each key to be calibrated fully down (bottom out) and releases it | — |
| stop | `21 00 00 00 18 10 00` |

Boards without `isAnyKeyCalibration` use `21 … 18 08 00` (start) / `21 … 18 08 01` (stop) and the user presses
all keys in turn. A third sub, `21 … 18 07 row col` (`reviseKey`, single-key calibration), exists in the
driver but is never called by the UI — **DO NOT USE**.

### 10.3 Calibration progress (Input, `b[5]` = `0x08` or `0x0F`) [JS]

| `b[6]` | meaning |
|---|---|
| 0 | calibration finished/stopped successfully |
| 1 | progress: `b[7..28]` = 22-byte transposed mask (section 2.5) of keys calibrated so far; colour them in the GUI |
| 2 or 4 | calibration failed |

Procedure for aulactl: start → show progress mask as keys complete → user clicks stop (or leaves the page,
in which case the official driver sends stop automatically) → wait for `b[6] = 0`. Never leave the board in
calibration mode: send the stop frame on every exit path. The official UI also asks for recalibration after a
switch-type change.

Never exercised on the wire by anyone (no capture); [JS] only.

---

## 11. Performance, reset, gamepad — `0x14`

`b[4]` is a constant `0x01`, not a length.

### 11.1 Win-lock options + NKRO/6KRO — WRITE [CAP]

```
b[0]=14 b[1]=00 b[4]=01
b[12] 1 = when win-lock is on, disable the Windows key
b[13] 1 = … disable Shift+Tab
b[14] 1 = … disable Alt+F4
b[15] 1 = … disable Alt+Tab
b[18] 1 = 6-key rollover, 0 = NKRO (only meaningful if isAllAnd6keySwitch; WIN 60 HE: always send the current value from 0x01 b[22])
```

ACK `14 00 00 00 00 00`. Current value: `0x01` reply `b[20]`, bits 0..3 = `b[12]`, `b[13]`, `b[14]`, `b[15]`
(note: bit 2 = Alt+F4, bit 3 = Alt+Tab). Win-lock itself is toggled on the keyboard (Fn function / keymap
entry `11 02`); these flags only choose what it blocks.

### 11.2 Factory reset — `14 00 00 00 01 01` — WRITE, DESTRUCTIVE [JS]

Resets the keyboard's stored settings. The official "Reset" button sends, in order: this frame, then
`21 … 18 06` (reset triggers), then `26 00` (only a read, although the driver calls it `resetDeadBand`), then
SOCD power off (`24 03 … 00`), then re-reads everything. Never observed on the wire. Only send after explicit user confirmation.

### 11.3 Gamepad mode — `14 03 00 00 01 m` — DO NOT USE on WIN 60 HE

m = 0 keyboard, 1 Xbox, 2 classical; the board restarts. `isGamePad = 0` on this board; the official UI sends
nothing when gamepad is clicked [CAP].

### 11.4 Sleep timers — `14 01` / `14 02` — not applicable

Wireless boards only (PIDs 0xC370/0xC371): write `14 01 … b[8..9]` light-sleep s (BE), `b[10..11]`
keyboard-sleep s (BE); read `14 02` → same offsets. Not used for WIN 60 HE.

---

## 12. SOCD ("PRCS") — `0x24`

Supported when `0x01` reply `b[14] == 1` (WIN 60 HE: yes).

### 12.1 Power

* **READ** `24 02` → `24 02 00 00 06 on x x x hk1 hk2` [DEV]. `on` = 1 enabled. `hk1`/`hk2` are HID-style
  codes the UI only displays as the on-board toggle hotkey; owner's board `fa e5` = **Fn + R-Shift**.
* **WRITE** `24 03 00 00 01 on` [CAP]. ~~ACK `24 00 …`~~ — on the owner's board the write takes effect but
  **no ACK** is sent [DEV 2026-11-01] (pair pages `24 00 pp` are ACKed). Don't wait for one: verify by reading
  `24 02` (`b[5]` = on).

### 12.2 Pairs

Up to **20 entries** of 4 bytes, sent as 2 pages × 40 bytes (10 entries each).

* **READ** `24 01` → 2 pages `24 01 00 pp 28 data…` [DEV]. Owner's board: empty.
* **WRITE** `24 00 00 pp 28 data…` for pp = 0, 1 [CAP]. ACK `24 00 …`.

Entry `[present, model, key1_hid, key2_hid]`:

* `present` = 1 on write. On read the driver accepts an entry only if byte 0 ≠ 0 and the entry is not
  `07 ff ff ff`; the capture read back `00 01 04 07` for a pair it had written with present = 1, so the read
  semantics of byte 0 are [?]. Entries are packed from the start; unused entries zero.
* `model` (0-based): 0 = whichever key was pressed first is released ("last input wins"), 1 = key 1 overrides
  key 2, 2 = key 2 overrides key 1, 3 = the later key is ignored and only releases the first.
  (UI: "Model1: The key that triggered first is interrupted / Model2: Key1 interrupt the triggering of Key2 /
  Model3: Key2 interrupt the triggering of Key1 / Model4: Later triggered key not executed and only interrupt
  the first triggered key".)
* keys are **HID usages**, not key indices (A + D = `04 07`). Non-US layouts: the driver writes `0x32` instead
  of `0x31` for key 2. Example [CAP]: A/D, model 2 → `01 01 04 07`.

---

## 13. Profiles, resets, misc

* Profiles: host-side only (section 9.9). Five slots in the official UI: "Default" and 1..4.
* Full reset: section 11.2. Trigger reset: section 9.8.
* No onboard macro recording, no battery, no side LEDs, no rotary knob on this board.

---

## 14. Physical key layout (61 keys)

Geometry is the official GUI's (`config/keys/SI2825HEARGB.json`, identical for `SI2825KZHEARGB`): canvas
587 × 217 px, 1 u = 38 px pitch, keys drawn 3 px narrower than their pitch. `x (u)` = `(x_px − 10) / 38`,
`width (u)` = `(w_px + 3) / 38`. "Fn-fixed" = in the `0x0E` mask on the owner's board (cannot be remapped in
the Fn layer). "default entry" = the base-layer keymap entry the official driver writes.

| index | matrix row | matrix col | label | code | default entry | visual row | x (u) | width (u) | x px | y px | w px | Fn-fixed |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 22 | 1 | 0 | `Esc` | Escape | `00 29 00 00` | 0 | 0.00 | 1.00 | 10 | 15 | 35 | yes |
| 23 | 1 | 1 | `1!` | Digit1 | `00 1e 00 00` | 0 | 1.00 | 1.00 | 48 | 15 | 35 | yes |
| 24 | 1 | 2 | `2@` | Digit2 | `00 1f 00 00` | 0 | 2.00 | 1.00 | 86 | 15 | 35 | yes |
| 25 | 1 | 3 | `3#` | Digit3 | `00 20 00 00` | 0 | 3.00 | 1.00 | 124 | 15 | 35 | yes |
| 26 | 1 | 4 | `4$` | Digit4 | `00 21 00 00` | 0 | 4.00 | 1.00 | 162 | 15 | 35 | yes |
| 27 | 1 | 5 | `5%` | Digit5 | `00 22 00 00` | 0 | 5.00 | 1.00 | 200 | 15 | 35 | yes |
| 28 | 1 | 6 | `6^` | Digit6 | `00 23 00 00` | 0 | 6.00 | 1.00 | 238 | 15 | 35 | yes |
| 29 | 1 | 7 | `7&` | Digit7 | `00 24 00 00` | 0 | 7.00 | 1.00 | 276 | 15 | 35 | yes |
| 30 | 1 | 8 | `8*` | Digit8 | `00 25 00 00` | 0 | 8.00 | 1.00 | 314 | 15 | 35 | yes |
| 31 | 1 | 9 | `9(` | Digit9 | `00 26 00 00` | 0 | 9.00 | 1.00 | 352 | 15 | 35 | yes |
| 32 | 1 | 10 | `0)` | Digit0 | `00 27 00 00` | 0 | 10.00 | 1.00 | 390 | 15 | 35 | yes |
| 33 | 1 | 11 | `-_` | Minus | `00 2d 00 00` | 0 | 11.00 | 1.00 | 428 | 15 | 35 | yes |
| 34 | 1 | 12 | `=+` | Equal | `00 2e 00 00` | 0 | 12.00 | 1.00 | 466 | 15 | 35 | yes |
| 36 | 1 | 14 | `Back` | Backspace | `00 2a 00 00` | 0 | 13.00 | 2.00 | 504 | 15 | 73 |  |
| 44 | 2 | 0 | `Tab` | Tab | `00 2b 00 00` | 1 | 0.00 | 1.50 | 10 | 53 | 54 |  |
| 45 | 2 | 1 | `Q` | KeyQ | `00 14 00 00` | 1 | 1.50 | 1.00 | 67 | 53 | 35 |  |
| 46 | 2 | 2 | `W` | KeyW | `00 1a 00 00` | 1 | 2.50 | 1.00 | 105 | 53 | 35 | yes |
| 47 | 2 | 3 | `E` | KeyE | `00 08 00 00` | 1 | 3.50 | 1.00 | 143 | 53 | 35 |  |
| 48 | 2 | 4 | `R` | KeyR | `00 15 00 00` | 1 | 4.50 | 1.00 | 181 | 53 | 35 |  |
| 49 | 2 | 5 | `T` | KeyT | `00 17 00 00` | 1 | 5.50 | 1.00 | 219 | 53 | 35 |  |
| 50 | 2 | 6 | `Y` | KeyY | `00 1c 00 00` | 1 | 6.50 | 1.00 | 257 | 53 | 35 |  |
| 51 | 2 | 7 | `U` | KeyU | `00 18 00 00` | 1 | 7.50 | 1.00 | 295 | 53 | 35 |  |
| 52 | 2 | 8 | `I` | KeyI | `00 0c 00 00` | 1 | 8.50 | 1.00 | 333 | 53 | 35 | yes |
| 53 | 2 | 9 | `O` | KeyO | `00 12 00 00` | 1 | 9.50 | 1.00 | 371 | 53 | 35 | yes |
| 54 | 2 | 10 | `P` | KeyP | `00 13 00 00` | 1 | 10.50 | 1.00 | 409 | 53 | 35 | yes |
| 55 | 2 | 11 | `[{` | BracketLeft | `00 2f 00 00` | 1 | 11.50 | 1.00 | 447 | 53 | 35 | yes |
| 56 | 2 | 12 | `]}` | BracketRight | `00 30 00 00` | 1 | 12.50 | 1.00 | 485 | 53 | 35 | yes |
| 58 | 2 | 14 | `\\|` | Backslash | `00 31 00 00` | 1 | 13.50 | 1.50 | 523 | 53 | 54 |  |
| 66 | 3 | 0 | `Caps` | CapsLock | `00 39 00 00` | 2 | 0.00 | 1.74 | 10 | 91 | 63 |  |
| 68 | 3 | 2 | `A` | KeyA | `00 04 00 00` | 2 | 1.74 | 1.00 | 76 | 91 | 35 | yes |
| 69 | 3 | 3 | `S` | KeyS | `00 16 00 00` | 2 | 2.74 | 1.00 | 114 | 91 | 35 | yes |
| 70 | 3 | 4 | `D` | KeyD | `00 07 00 00` | 2 | 3.74 | 1.00 | 152 | 91 | 35 |  |
| 71 | 3 | 5 | `F` | KeyF | `00 09 00 00` | 2 | 4.74 | 1.00 | 190 | 91 | 35 |  |
| 72 | 3 | 6 | `G` | KeyG | `00 0a 00 00` | 2 | 5.74 | 1.00 | 228 | 91 | 35 |  |
| 73 | 3 | 7 | `H` | KeyH | `00 0b 00 00` | 2 | 6.74 | 1.00 | 266 | 91 | 35 |  |
| 74 | 3 | 8 | `J` | KeyJ | `00 0d 00 00` | 2 | 7.74 | 1.00 | 304 | 91 | 35 | yes |
| 75 | 3 | 9 | `K` | KeyK | `00 0e 00 00` | 2 | 8.74 | 1.00 | 342 | 91 | 35 | yes |
| 76 | 3 | 10 | `L` | KeyL | `00 0f 00 00` | 2 | 9.74 | 1.00 | 380 | 91 | 35 | yes |
| 77 | 3 | 11 | `;:` | Semicolon | `00 33 00 00` | 2 | 10.74 | 1.00 | 418 | 91 | 35 | yes |
| 78 | 3 | 12 | `" '` | Quote | `00 34 00 00` | 2 | 11.74 | 1.00 | 456 | 91 | 35 | yes |
| 80 | 3 | 14 | `Enter` | Enter | `00 28 00 00` | 2 | 12.74 | 2.26 | 494 | 91 | 83 |  |
| 88 | 4 | 0 | `L-Shift` | ShiftLeft | `00 e1 00 00` | 3 | 0.00 | 2.26 | 10 | 129 | 83 |  |
| 90 | 4 | 2 | `Z` | KeyZ | `00 1d 00 00` | 3 | 2.26 | 1.00 | 96 | 129 | 35 |  |
| 91 | 4 | 3 | `X` | KeyX | `00 1b 00 00` | 3 | 3.26 | 1.00 | 134 | 129 | 35 |  |
| 92 | 4 | 4 | `C` | KeyC | `00 06 00 00` | 3 | 4.26 | 1.00 | 172 | 129 | 35 |  |
| 93 | 4 | 5 | `V` | KeyV | `00 19 00 00` | 3 | 5.26 | 1.00 | 210 | 129 | 35 |  |
| 94 | 4 | 6 | `B` | KeyB | `00 05 00 00` | 3 | 6.26 | 1.00 | 248 | 129 | 35 |  |
| 95 | 4 | 7 | `N` | KeyN | `00 11 00 00` | 3 | 7.26 | 1.00 | 286 | 129 | 35 | yes |
| 96 | 4 | 8 | `M` | KeyM | `00 10 00 00` | 3 | 8.26 | 1.00 | 324 | 129 | 35 | yes |
| 97 | 4 | 9 | `,<` | Comma | `00 36 00 00` | 3 | 9.26 | 1.00 | 362 | 129 | 35 | yes |
| 98 | 4 | 10 | `.>` | Period | `00 37 00 00` | 3 | 10.26 | 1.00 | 400 | 129 | 35 |  |
| 99 | 4 | 11 | `/?` | Slash | `00 38 00 00` | 3 | 11.26 | 1.00 | 438 | 129 | 35 |  |
| 100 | 4 | 12 | `R-Shift` | ShiftRight | `00 e5 00 00` | 3 | 12.26 | 2.74 | 476 | 129 | 101 | yes |
| 110 | 5 | 0 | `L-Ctrl` | ControlLeft | `00 e0 00 00` | 4 | 0.00 | 1.32 | 10 | 167 | 47 |  |
| 111 | 5 | 1 | `L-Win` | MetaLeft | `00 e3 00 00` | 4 | 1.32 | 1.32 | 60 | 167 | 47 | yes |
| 112 | 5 | 2 | `L-Alt` | AltLeft | `00 e2 00 00` | 4 | 2.63 | 1.32 | 110 | 167 | 47 |  |
| 116 | 5 | 6 | `Space` | Space | `00 2c 00 00` | 4 | 3.95 | 5.79 | 160 | 167 | 217 | yes |
| 119 | 5 | 9 | `R-Alt` | AltRight | `00 e6 00 00` | 4 | 9.74 | 1.32 | 380 | 167 | 47 | yes |
| 120 | 5 | 10 | `Menu` | ContextMenu | `00 65 00 00` | 4 | 11.05 | 1.32 | 430 | 167 | 47 |  |
| 121 | 5 | 11 | `R-Ctrl` | ControlRight | `00 e4 00 00` | 4 | 12.37 | 1.32 | 480 | 167 | 47 | yes |
| 122 | 5 | 12 | `Fn` | KeyFn | `01 fa 00 00` | 4 | 13.68 | 1.32 | 530 | 167 | 47 | yes |

All other indices 0..131 are not keys: zero in every table, never set in masks.

---

## 15. Disagreements and corrections

| # | claim (where) | verdict |
|---|---|---|
| 1 | Aether `CLAUDE.md`: travel stream is "cmd33 sub 5" | Wrong label. The stream is `b[5] = 0x01`; sub 5 is the per-key config read. Offsets are right. Also missing: the `b[4] = 3` "returned to rest" frame (10.1). |
| 2 | Aether capture notes: macro header byte 1 = slot index duplicated, byte 5 = repeat count | Official JS: byte 1 = play mode (0..3), bytes 4..5 = BE repeat count. The capture is consistent with the JS (modes 0 and 1, counts 1 and 3). |
| 3 | Aether capture notes: device dead-band default is 2/2 | A never-written board reads 0/0 (owner's board and the capture's own first mount). 2 is only the UI slider default. Aether's builder default 4/5 is also not the device default. |
| 4 | Aether capture notes: `09 20` meaning unknown | Official JS: reply `b[1]` = active custom slot (Custom1/Custom2). |
| 5 | Aether capture notes: `0x0E` not decoded | 22-byte mask of keys whose Fn-layer function is firmware-fixed (6.2). |
| 6 | Aether `protocol.py`: win-lock bits 2/3 order | Wire `b[14]` = Alt+F4, `b[15]` = Alt+Tab (`0x01 b[20]` bit 2 = Alt+F4, bit 3 = Alt+Tab). |
| 7 | Official read parser for `0x0D` uses `subarray(5, len)` | Vendor bug; use `5 .. 5+len`. |
| 8 | Official macro read path maps mouse codes 0/1/2 → L/M/R while the write path writes 1/2/4 | Vendor inconsistency; follow the write path [?]. |
| 9 | Aether notes call `21 … 11` "read current onboard preset" and suggest profiles might exist | WIN 60 HE never answers it; profiles are host-side only. |
| 10 | Keymap read of a never-written board shows the Fn key as `01 00 00 00` | The official driver writes Fn as `01 fa 00 00`; both are accepted by the firmware (the capture board reads back `01 fa`). Write `01 fa`. |
| 11 | 2.4 / 12.1 (earlier draft): SOCD power write ACKed `24 00` | Real board 2026-11-01: no ACK for `24 03`; verify via `24 02`. |
| 12 | 7: one slot can be written alone | Real board 2026-11-01: single-slot writes are often dropped; write all 10 slots in order 0..9. |
| 13 | 5.5: a custom-slot write only changes the table | Real board 2026-11-01: it also switches the effect to Custom (mode 10, brightness 4, speed 3). |

---

## 16. Open questions

1. DKS phase values 2 vs 3: exact firmware semantics (editor-derived meaning given in section 8).
2. Which custom slot mode 10 displays and how it is chosen (inferred: last written / the `09 20` reply).
3. Meaning of `0x01` reply `b[19]` (= 1) and `21 sub 04` reply `b[8]` (= 1).
4. Second travel copy in trigger read/write (`b[8]`/`b[12]` read, `b[29]`/`b[33]` write) — always equal to the
   first in every observation; possibly a separate release point that the UI never sets differently.
5. SOCD read-back byte 0 semantics (12.2).
6. Speed direction: whether speed 0 is slowest or fastest (UI default 3, range 0..4).
7. Whether the heartbeat is needed for anything on a wired board (nothing observed without it).
8. Calibration, factory reset, trigger reset and the advanced-key payloads have never been seen on the wire.
   Test them only with the owner present.
9. Calibration notification shape (10.3): aulactl assumes `b[4]` = length of `b[5..]` as in other notifications,
   ignores `b[4] < 2` (ACK shape) and ignores a frame identical to the start request `21 00 00 00 18 0F 00`
   within 100 ms of sending it (it would otherwise read as `b[6] = 0` finished). Re-check against the first real
   calibration capture: what the board answers to start, and the `b[4]` of progress / finished frames.

---

## Appendix A — raw frames from the owner's board (2026-11-01, READ-only probe)

hidraw bytes, report ID `01` first, trailing zeros trimmed. Probe: `~/aula-re/scripts/probe.py` (allowlist of
READ frames; travel test always closed in `finally`). Full logs: `~/aula-re/scripts/probe-out/{reads,stream}.txt`.

```
OUT 01 0d
IN  01 0d 00 00 00 32 57 36 36 39 2c 33 34 2c 4b 42 2c 53 49 2c 53 49 32 38 32 35 4b 5a 48 45 41 52 47 42 2c 56 33 2e 31 37 2e 30 37
        "W669,34,KB,SI,SI2825KZHEARGB,V3.17.07"
IN  01 0d 00 00 01 32 41 70 72 20 31 35 20 32 30 32 36 2c 31 31 3a 32 30 3a 33 35 00 00 00 00 41 70 72 20
        "Apr 15 2026,11:20:35" NUL, then stale bytes

OUT 01 01
IN  01 01 00 00 00 11 00 00 00 00 00 00 00 00 00 01 00 00 7f 00 01 01

OUT 01 0e
IN  01 0e 00 00 00 16 02 22 0e 0a 02 02 22 12 1e 3e 0e 2e 3e 20 20 20

OUT 01 0a
IN  01 0a 00 00 00 12 02 01 03 09 04 00 0e 0f 06 07 0b 0c 10 08 a5 5a 04 04
OUT 01 02
IN  01 02 00 00 00 04 a5 5a 04 04

OUT 01 07 01
IN  01 07 00 00 00 0c 00 01 02 ff 00 ff
OUT 01 08 01
IN  01 08 00 00 00 0c 02 04 03 ff 00 00 00 00 00 00 01

OUT 01 09 20
IN  01 09
OUT 01 09 80
IN  01 09 80 00 00 36
IN  01 09 80 00 01 36
IN  01 09 80 00 02 36 00 … 00 ff                      (idx 46 = W red)
IN  01 09 80 00 03 36 00 … ff 00 00 ff 00 00 ff        (idx 68-70 = A S D red)
IN  01 09 80 00 04 36
IN  01 09 80 00 05 36
IN  01 09 80 00 06 36
IN  01 09 80 00 07 12

OUT 01 18 80
IN  01 18 80 00 00 38            … pages 01..07 all zero …
IN  01 18 80 00 08 38 00×40 01   (idx 122 Fn = 01 00 00 00)
IN  01 18 80 00 09 18
OUT 01 18 82
IN  01 18 82 00 00 38 … 01 18 82 00 09 18   (all zero)

OUT 01 19 80              (same for 19 81 … 19 89)
IN  01 19 80 00 00 3a
IN  01 19 80 00 01 3a
IN  01 19 80 00 02 3a
IN  01 19 80 00 03 3a
IN  01 19 80 00 04 18     (all zero)

OUT 01 22 80
IN  01 22 80 00 00 28 … 01 22 80 00 27 28   (40 frames, all zero)

OUT 01 24 02
IN  01 24 02 00 00 06 00 00 00 00 fa e5
OUT 01 24 01
IN  01 24 01 00 00 28
IN  01 24 01 00 01 28

OUT 01 25
IN  01 25 00 00 00 06 00 04 01 02 03 05
OUT 01 25 02
IN  01 25 02 00 00 3a 01×58
IN  01 25 02 00 01 3a 01×58
IN  01 25 02 00 02 10 01×16 (stale 01s follow)

OUT 01 26
IN  01 26 00 00 00 3a … 01 26 00 00 04 20   (5 pages, all zero)

OUT 01 21 00 00 00 18 04
IN  01 21 00 00 00 06 04 54 01 01 01 08
OUT 01 21 00 00 00 01 09
IN  01 21 00 00 00 02 09 08
OUT 01 21 00 00 00 00 11
(no reply)
OUT 01 21 00 00 00 18 05 01        (idx 22, Esc)
IN  01 21 00 00 00 0c 05 0c 64 64 32 32 00 00 00 00 01
OUT 01 21 00 00 00 18 05 05 0c     (idx 122, Fn)
IN  01 21 00 00 00 0c 05 0c 64 64 32 32 00 00 00 00 05 0c
                                   (all 61 keys identical apart from row/col)
```

Travel test (owner typing during the 6 s window):

```
OUT 01 21 00 00 00 18 02 3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e
IN  01 21 00 00 00 01                                   ACK
IN  [3028.5 ms] 01 21 00 00 00 05 01 05 00 6e          idx 110 L-Ctrl, 1.10 mm
IN  [3035.4 ms] 01 21 00 00 00 05 01 05 00 a9          1.69 mm
IN  [3050.7 ms] 01 21 00 00 00 05 01 05 00 54 01       3.40 mm (bottom)
…
IN  [3366.7 ms] 01 21 00 00 00 05 01 02 02 57          idx 46 W, 0.87 mm (releasing)
IN  [3373.1 ms] 01 21 00 00 00 03 01 02 02 00 00 ff ff 27 0a ff 09 …   idx 46 at rest (len 3, stale tail)
IN  [3790.1 ms] 01 21 00 00 00 05 01 02 05 00 00       idx 49 T, 0.00 mm
IN  [4182.5 ms] 01 21 00 00 00 03 01 02 05 08 00 ff ff …               idx 49 at rest (stale 08 00)
OUT 01 21 00 00 00 18 03
IN  01 21 00 00 00 01                                   ACK
```
