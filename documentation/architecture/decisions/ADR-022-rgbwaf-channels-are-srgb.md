# ADR-022: RGBWAF channels are sRGB above the wire and linear on it

Status: Accepted
Date: 2026-08-20

## Context

IEC 62386-209 gives RGBWAF no colour space: it defines dim levels 0..254, a control
type, and what each combination does. What it does define is arithmetic. Under
normalised colour control (`0x80`, which the colour write path asserts where permitted)
§9.1.2 makes each channel a **linear** fraction of arc power, scaled against
`MAX(R, G, B, W, A, F)`. Every surface that produces a colour — a browser colour input,
Home Assistant's `rgb_color`, the device card's swatch, a stored scene row — is
gamma-encoded **sRGB**, where code 120 is 18 % of full output, not 47 %. Passing sRGB
bytes to the wire unchanged over-drives every mid-tone: the colour arrives desaturated
and pulled toward its weakest channel. Primaries, white and black are fixed points of any
monotone transfer, which is why the error hides until someone picks a mid-tone.

## Decision

**The product speaks sRGB; the wire speaks linear dim levels; the conversion happens at
the wire boundary, in both directions, from one definition.**

- `srgb_channel_to_dim_level` / `dim_level_to_srgb_channel`
  (`dali2rust-domain/src/dali/devices/dt8_color.rs`) are that definition: two 256-entry
  `const` tables, because the domain crate carries no floating-point maths, the values
  must be bit-stable across targets, and the encoder sits on the interactive write path.
  Tests re-derive every entry from the piecewise sRGB transfer function.
- **Encode at the one write point** — the RGB and WAF staging shared by live writes and
  the scene programmer. The DALI ceiling of 254 folds into the encoder.
- **Decode at all three read points** — the DT8 attribute-read projection, the scene
  `REPORT` projection, and the sniffer translator. The registry then holds one space.
- **Nonzero input floors at dim level 1.** Otherwise a low but nonzero colour encodes to
  all zeros, and §9.1.2's `MAX = 0` branch drives every channel at the arc level — full
  white against a near-black swatch. The floor also keeps "wire 0 ⇔ product 0" a
  biconditional, which the scene MASK rule relies on.
- **Comparisons run through the encoder, never the decoder.** A convergence check encodes
  the desired value and compares in wire space, the same doctrine as `kelvin_to_mirek`.
  The encoder is many-to-one at the dark end, so decoding an observation first would
  leave a converged scene row dirty for ever.
- **The inverse law is stated as it is.** A global `forward(inverse(w)) == w` is
  impossible: above the knee the encoder's slope exceeds 1 and its image has holes. What
  holds, and what the tests pin, is `forward(inverse(forward(s))) == forward(s)` for every
  `s`, an exact round trip above the knee, off-image levels decoding to the nearest
  attainable one, monotone tables, `0 ↔ 0` and MASK decoding to MASK.

### Rejected alternatives

- **Converting in the Home Assistant bridge only** — fixes one door and leaves the UI,
  the scene matrix and REST wrong for mid-tones, while making the meaning of `rgb` depend
  on the client.
- **A setting to choose the transfer function** — two mechanisms for one effect.
- **Computing the curve at runtime** — floating point on the interactive path for a
  function with 256 inputs.

## Consequences

- A stored colour is an sRGB byte triple and lights as sRGB describes it; primaries,
  white and black are fixed points of the conversion.
- The dark end is coarse: fourteen sRGB codes share dim level 1, and writing a very dark
  value reads back one step different — visible only in the registry, never as a dirty
  scene row, because convergence compares through the encoder.
- Home Assistant needs no conversion of its own; the bridge passes the product space
  through.
- The gear model holds wire values and is not converted: it is the gear.
- BDD expectations are written as literals, not computed from the table, so the scenarios
  stay an independent oracle.
- Open: whether a vendor applies the IEC 62386-207 dimming curve to the RGBWAF channels
  as well as to the arc level; by §9.1.2 it should sit in the common arc-power factor and
  leave the colour ratios untouched.
