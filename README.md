# discord-wavenet

A very stupid Discord bot that will play TTS into a voice channel using audio synthesized by the full might of Google Wavenet.
Millions of dollars of R&D, who knows how many man-hours, the sweat and tears of countless ML PH.ds, for this.

## Why?

"I can't understand anything Hawking says, and you keep making it say really dumb things." -- [@chrissprague](https://github.com/chrissprague) 2021

## Building

You will need:

* Rust 1.48 or newer
* Opus development libraries installed (`libopus-dev` on Debian-alikes, `opus-devel` on RHEL-alikes)
* ffmpeg

After that you can simply run `cargo build` and it should all work itself out naturally.

## Running

You will need your Discord bot API token as well as a Google API token. You will also need to make sure you have a credit
card in your Google cloud developer console and that you have a text to speech project set up and ready.

Once you have those two things, it's very easy to run the bot. The tokens are passed into the bot via environment variables.

### Via the command line

```sh
export DISCORD_TOKEN=<Discord token goes here>
export GOOGLE_API_CREDENTIALS=<Path to file containing Google API JSON goes here>
cargo run --release
```

### Via the Docker container

```sh
docker run -e "DISCORD_TOKEN=<Discord token goes here>" -e "GOOGLE_API_CREDENTIALS=<Path to file containing Google API JSON goes here>" --rm -it ghcr.io/sriramanujam/discord-wavenet:latest
```

### Advanced usage

For those of you who would like to deploy this into a Kubernetes cluster, rest assured that I am one of you! Therefore,
the Docker container should run just fine in k8s, provided you pass it the environment variables it requires. As an added
bonus, the bot implements liveness and readiness checks on a localhost-only webserver for your convenience.

```sh
# as an example to guide you, curl is not actually shipped inside the container. You can use the httpGet check instead.
curl -o- http://localhost:8080/livez?verbose
curl -o- http://localhost:8080/readyz?verbose
```

## Doesn't using Wavenet cost money?

Yes, but the first million characters a month are free. This is why I have no intention of hosting this bot publicly somewhere. It would most likely bankrupt me. Anyone interested in hosting the bot on their own Discord servers should judge very carefully whether they will be able to consistently stay under the 1 million character limit.

Having said that, though, a million characters is a lot. For reference, Mary Shelley's _Frankenstein_ is 448,821 characters long. You could get the bot to read out Frankenstein twice and still have 102,358 characters left over every month, forever, without paying Google a single cent. I wouldn't be too worried.
