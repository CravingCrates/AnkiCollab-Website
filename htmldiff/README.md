# htmldiff

Rust port of [myobie/htmldiff](https://github.com/myobie/htmldiff).

[![Linux build status](https://travis-ci.org/seikichi/htmldiff.svg)](https://travis-ci.org/seikichi/htmldiff)
[![Windows build status](https://ci.appveyor.com/api/projects/status/github/seikichi/htmldiff?svg=true)](https://ci.appveyor.com/project/seikichi/htmldiff)
[![Crates.io](https://img.shields.io/crates/v/htmldiff.svg)](https://crates.io/crates/htmldiff)

## Installation

### Cargo

```sh
$ cargo install htmldiff
```

### Manual

You can download prebuilt binaries in the
[releases section](https://github.com/seikichi/htmldiff/releases),
or create from source.

```sh
$ git clone https://github.com/seikichi/htmldiff.git
$ cd htmldiff
$ cargo build --release
```

## Run

```sh
$ cat old.html
<p>Hello, world!</p>
$ cat new.html
<p>Hello, seikichi!</p>
$ htmldiff old.html new.html
<p>Hello, <del>world!</del><ins>seikichi!</ins></p>
```

## Use as Library

Add the following to your Cargo.toml file:

```toml
[dependencies]
htmldiff = "0.1"
```

Next, call `htmldiff::htmldiff` function in your code:

```rust
extern crate htmldiff;

fn main() {
    let old = "<p>Hello, world!</p>";
    let new = "<p>Hello, seikichi!</p>";
    println!("{}", htmldiff::htmldiff(old, new));
}
```

## License

MIT

## Alternatives

- [HtmlDiff - W3C Wiki](https://www.w3.org/wiki/HtmlDiff)
