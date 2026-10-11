# libertas-samples
Sample code for Libertas Platform.

The Tax App has moved to its own repository:
[LibertasIoT/libertas-tax](https://github.com/LibertasIoT/libertas-tax), including
its source, tests, public development guidance and coverage audit.
Maintainer instructions and investigation evidence remain private.

Platform documentation is published only in
[libertas-doc](https://github.com/LibertasIoT/libertas-doc). Start with its
[CodeBot entry](https://github.com/LibertasIoT/libertas-doc/blob/main/CODEBOT.md)
and [Rust card](https://github.com/LibertasIoT/libertas-doc/blob/main/codebot/rust.md)
for authoring; local maintainer instructions are not part of the published tree.

Persistent samples declare both unions under the
[storage schema specification](https://github.com/LibertasIoT/libertas-doc/blob/main/reference/protocols.md#storage-schema-declarations)
and use the same
[transactional KV API](https://github.com/LibertasIoT/libertas-doc/blob/main/reference/protocols.md#transactional-key-value-api).
The [weather/sprinkler example](https://github.com/LibertasIoT/libertas-doc/blob/main/examples/weather-sprinkler.md)
explains their responsibilities and links to the portable storage rules.

## Rust samples

- [Libertas chart demo](rust/libertas-chart_demo/) — an interactive endpoint
  gallery covering every Libertas chart mark, composition, scale family, guide
  source, and encoding channel.
