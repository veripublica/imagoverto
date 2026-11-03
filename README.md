# imagoverto

imagoverto converts a still image between WebP, AVIF, JPEG XL, JPEG and PNG.

I wrote it for two opposite needs. Some e-readers can't show WebP, AVIF or JPEG XL, so those images need a way back to JPEG or PNG. [epublift](https://github.com/ePubLift/epublift), another of my tools, will use it to go the other way. Neither direction comes first, and imagoverto itself knows nothing about books.

The easiest way to use it is [the web page](https://veripublica.github.io/imagoverto/). Your image stays in your browser and nothing is uploaded.

For the command line, download a binary from the [latest release](https://github.com/veripublica/imagoverto/releases/latest), or install it with `cargo install --locked imagoverto`. Then run:

```sh
imagoverto -i cover.webp --image-format jpeg --quality 85
```

This writes `cover.jpg` next to your image and doesn't touch the original. Leave out `--quality` and nothing is lost, except that JPEG has no lossless mode, so a JPEG target always needs one.

One thing is missing for now: it reads AVIF but doesn't write it. The AVIF encoder I trust has the fixes it needs only on git so far, and crates.io won't take a git dependency. AVIF output comes back once that encoder has a release.

imagoverto only converts. It won't pick a format for you, resize an image or decide whether the result is worth keeping. The options, the JSON report, the library, the npm package and my measurements are in [AGENTS.md](https://github.com/veripublica/imagoverto/blob/main/AGENTS.md).

The images you convert are yours, and you can use the tool commercially for free. The code is AGPL-3.0-only, with no commercial licence; [LICENSING.md](https://github.com/veripublica/imagoverto/blob/main/LICENSING.md) says why. I can't accept pull requests yet, and [CONTRIBUTING.md](https://github.com/veripublica/imagoverto/blob/main/CONTRIBUTING.md) explains that too.
