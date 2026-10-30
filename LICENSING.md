# Licensing, in plain terms

`imagoverto` is licensed under **AGPL-3.0-only** ([`LICENSE`](./LICENSE)).

This page answers the questions people actually ask before adopting it. The
GPL-compatibility reasoning, clause by clause, lives in one shared FAQ for the
veripublica tools:

**→ [Licensing FAQ](https://github.com/veripublica/epubsana/blob/main/LICENSING-FAQ.md)**

That FAQ is written for the tools that also offer a commercial license. Its
answers about using the tools, about plugins and about GPL-3.0 software apply
here unchanged. `imagoverto` differs in one way, explained below: it offers no
commercial license.

---

## Are the images I convert covered by your license?

**No. Unconditionally.**

The license governs this software, not the data it processes. Converting an
image with `imagoverto` no more licenses that image than a compiler licenses
the program it compiles. Sell the result, publish it, keep it. You owe us
nothing and you need no permission.

## I convert images commercially. Do I need anything from you?

**No.** Running `imagoverto` on your own images, on customers' images, inside
your company, as often as you like, for money, needs no permission. There is no
hobbyist/commercial distinction in how you may *use* it.

## Why AGPL-3.0-only, with no commercial option?

The other veripublica tools are AGPL-3.0-only **or** a commercial license.
`imagoverto` is not, for a concrete reason: its WebP, AVIF and JPEG XL support
comes from Imazen's `zen*` codecs, which are themselves
`AGPL-3.0-only OR LicenseRef-Imazen-Commercial`. A veripublica commercial
license could never cover Imazen's code, so it would promise something it
cannot deliver.

If you want to embed `imagoverto` in a closed-source product you distribute, or
run a modified version as a network service without publishing your changes,
write to us. The copyright holder can grant other terms for `imagoverto`'s own
code. For the codecs you would also need terms from
[Imazen](https://github.com/imazen).

## What is it built on, and under which licenses?

The codecs that matter for licensing are these. The full list is in
`Cargo.lock`, and `cargo tree` shows how each one gets pulled in.

| Crate | What for | License |
|---|---|---|
| `zenwebp` | WebP read and write | AGPL-3.0-only OR LicenseRef-Imazen-Commercial |
| `zenjxl`, `jxl-encoder` (with `jxl-encoder-simd`, `zenjpeg`) | JPEG XL read and write, JPEG → JPEG XL recompression | AGPL-3.0-only OR LicenseRef-Imazen-Commercial |
| `zenravif`, `zenrav1e` | AVIF write | AGPL-3.0-only OR LicenseRef-Imazen-Commercial |
| `rav1d-safe` | AVIF read (the AV1 data) | AGPL-3.0-only OR LicenseRef-Imazen-Commercial |
| `zenavif-parse` | AVIF read (the container) | MPL-2.0 |
| `zenavif-serialize` | AVIF write (the container) | BSD-3-Clause |
| `image` | JPEG and PNG read and write | MIT OR Apache-2.0 |
| `moxcms` | colour profiles | BSD-3-Clause OR Apache-2.0 |

The rest are small permissive crates: MIT, Apache-2.0, BSD, Zlib, CC0 or
Unlicense, most of them dual-licensed. All of them are compatible with
AGPL-3.0-only.

**Three of these come from git, not crates.io.** `zenravif` (from
[imazen/cavif-rs](https://github.com/imazen/cavif-rs)), `zenrav1e` and
`zenavif-serialize` are taken from Imazen's repositories, each pinned to one
commit. The released `zenravif` 0.1.3 damages some real images, and its
lossless mode loses pixels. The unreleased 0.2.0 fixes both. Their licenses
are the same as above. imagoverto moves to the crates.io release when it is
published. A project that depends on imagoverto needs the same
`[patch.crates-io]` line for `zenavif-serialize`; the README shows it.

`ravif`, the BSD-licensed AVIF encoder that `zenravif` was forked from, is
not used. It was measured as an alternative and not taken.

## Can a Sigil or calibre plugin use `imagoverto`? Can GPL-3.0 software?

Yes to both. The reasoning is in the
[Licensing FAQ](https://github.com/veripublica/epubsana/blob/main/LICENSING-FAQ.md),
which walks through GPLv3 §13 and AGPLv3 §13 and quotes both.

## Contributing

External contributions are not being accepted yet. To keep the option of
granting other terms, the copyright holder must hold full copyright, so a CLA
has to exist before any external code can be merged. That mechanism is not
built yet. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).

---

## The commitment, and the disclaimer

I am not a lawyer, and nothing here is legal advice. Where this page and
[`LICENSE`](./LICENSE) could be read differently, **`LICENSE` governs.**

What is *not* hedged is the commitment itself, which I can make plainly as the
copyright holder: **I will not come after users, plugin authors, or editor
projects.** The AGPL is here because work of mine was once closed and
commercialised by someone else and I got nothing back. It is aimed at that, and
at nothing that a person converting their own images is doing.

Questions: baris@kayadelen.com
