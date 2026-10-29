# imagoverto

A still-image converter for WebP, AVIF, JPEG XL, JPEG and PNG: a Rust library,
and a command-line tool for people and scripts who want to convert images
without writing code. Part of the [veripublica](https://github.com/veripublica)
tools. The name is Latin: *imago*, image, and *verto*, to turn.

**Status:** being set up. Nothing is released yet.

## Scope

`imagoverto` works on **image file formats**. Video containers are out of
scope: a video converter would be a separate project. Animated images use
image formats, so they belong here if they are ever supported.

## Licence

AGPL-3.0-only ([LICENSE](LICENSE)). Converting an image with `imagoverto` puts
no license on the result. See [LICENSING.md](LICENSING.md) for what this means
in practice, and why there is no commercial option.
