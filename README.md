# [Talkboy](https://en.wikipedia.org/wiki/Talkboy)

Record and playback HTTP sessions with the goal of isolating testing web APIs.

# Usage

## Global options

* `-d, --recording-dir RECORDING_DIR`: directory to store/load sessions from.  Default `$CWD/recordings`.

## Recording mode

```
talkboy-record 
Start a proxy to record HTTP sessions

USAGE:
    talkboy record [OPTIONS] (--config CONFIG | [--addr ADDR] [--port PORT] [--ignore STATUS_CODES] PROJECT URL)

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -a, --addr <ADDR>              Address to listen on [default: 127.0.0.1]
    -c, --config <CONFIG>          Use config file to specify proxy options
    -i, --ignore <STATUS_CODES>    Comma-delimited status codes to ignore and not record responses for
    -p, --port <PORT>              Port to listen on [default: 8080]

ARGS:
    <PROJECT>    Project name used to group HTTP sessions
    <URL>        URL to proxy requests to
```

## Playback mode

```
talkboy-playback 
Start a server to play back recorded HTTP sessions

USAGE:
    talkboy playback [OPTIONS] (--config CONFIG | [DELAY_OPTION] [--addr ADDR] [--port PORT] PROJECT)

FLAGS:
    -h, --help              Prints help information
        --original-delay    Respond to requests with the original latency
    -V, --version           Prints version information

OPTIONS:
    -a, --addr <ADDR>        Address to listen on [default: 127.0.0.1]
    -c, --config <CONFIG>    Use config file to specify playback options
    -D, --delay-ms <MS>      Introduce a static delay to each request
    -p, --port <PORT>        Port to listen on [default: 8080]

ARGS:
    <PROJECT>    Project name used to group HTTP sessions
```

## Example

1. Start talkboy recording with `talkboy record myproject http://my-api.example.com`
2. Make requests to `localhost:8080` as though it were `my-api.example.com`.
3. `Ctrl+C` talkboy.  The requests and responses (In [HAR](https://w3c.github.io/web-performance/specs/HAR/Overview.html) format) will be in `$CWD/recordings/myproject`.
4. Run `talkboy playback myproject`
5. Make the same requests to `localhost:8080` and the recorded responses will be served instead of going to `my-api.example.com`.  Requests that weren't previously seen will return 404s. Isolated testing with real data!

# Config file 

Talkboy can also be driven by a toml file that defines recording and playback options.  There are two advantages to this over CLI mode: 
1. Multiple projects can record or playback simultaneously
2. Configuration can be version-controlled alongside the recordings

## Config Format

```toml
# Required, can be specified multiple times for multiple projects
[[project]]
# Required
name = "foo"
# Optional.  Address to bind to, defaults to 127.0.0.1
addr = "127.0.0.1"
# Optional. Omitted port numbers will start at 8080 and increment by 1
port = 8080

# Optional.  If absent, `talkboy playback` will not start a playback server for this project
[project.playback]
# Optional. method can be one of "None", "Original", or "Static" with a `millis` argument
delay = { method = "None" }

# Optional. If absent, `talkboy record` will not start a recording proxy for this project
[project.proxy]
# Required. URI to proxy requests to while in record mode
uri = "https://api1.example.com"
# Optional.  List of status codes to not record responses for
ignored_status_codes = [ 503 ]

[[project]]
name = "bar"
port = 8081

[project.playback]
delay = { method = "Static", millis = 500 }

[project.proxy]
uri = "https://api2.example.com"
```

# Updating Recordings

Recordings have a hash associated with them based on the following pieces of information from the *Request*:
* HTTP Method
* Path and query string
* HTTP Version
* POST body, if any

The full hash can be found in the `comment` field of the request entry in the corresponding HAR file.

The file name of the HAR archive is formatted according to `[HTTP Method].[up to 20 characters of the path].[the first 8 characters of the request hash as above].json`. The request path will be normalized to replace everything but `[a-zA-Z0-9_-\.]` with `-`.

This is done so that updating the response is as easy as re-running `talkboy record` with the same project.  If the same request results in a new body, the old recording will be overwritten.

# Divergence from the HAR spec

## PostData encoding

HAR 1.2 provides no way of specifying a POST body as anything other than key/value form data or plain text.  If a POST body contains non-UTF8 characters, talkboy will base64-encode the POST body, place it in the `text` field, and set the comment on the `PostData` object to `"base64"`.

There is a proposed HAR 1.3 draft that fixes this problem by adding an `encoding` field to `PostData`, which mirrors how the `Content` object works for Response bodies.

# Building

Requires a 2018-edition Rust compiler (version 1.32.0 used).

`cargo build` to build the debug version, or `cargo build --release` to build the release version.  Trace logging is enabled in the debug build.

# License

This project is licensed under either [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
