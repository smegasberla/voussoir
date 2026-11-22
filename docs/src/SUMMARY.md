# Summary

A Voussoir app is a Rust shell around a webview, with the frontend built by Bun.
The book goes from an empty directory to a signed installer, and spends its time
on the things that decide whether an app feels finished: the command contract,
the security policy, and what happens when something goes wrong.

It is written for someone meeting one half of this stack for the first time. Every
chapter opens with the theory behind what it describes, then the practice.

If you read nothing else, read [Concepts](concepts.md) and then
[Commands & payloads](ipc-contract.md). Everything else follows from them.

- [Concepts](concepts.md)
- [Installing Voussoir](install.md)
  - [Command-line reference](cli.md)
- [Your first app](first-app.md)
  - [How a project is laid out](layout.md)
  - [Configuring an app](configuration.md)
  - [The frontend](frontend.md)
- [Commands & payloads](ipc-contract.md)
  - [Typed commands](typed-commands.md)
  - [Errors the page can branch on](errors.md)
  - [The App builder, method by method](app-api.md)
- [Storage](storage.md)
  - [In-process state](state.md)
- [Plugins](plugins.md)
- [Native integration: menu, tray, dialogs](native-integration.md)
- [Security model](security.md)
  - [The two origins](origins.md)
  - [Per-window allowlists](allowlists.md)
- [Multi-window](multi-window.md)
  - [Lifecycle hooks](lifecycle.md)
- [Hot reload in development](hot-reload.md)
- [Testing](testing.md)
- [Packaging](packaging.md)
  - [Updating the app](updater.md)
- [Size and speed](size.md)
  - [Android and iOS](mobile.md)
- [Operations](operations.md)
  - [Diagnostics](diagnostics.md)
  - [Benchmarks](benchmarks.md)
- [Platform support](platforms.md)
- [Compatibility policy](compatibility.md)
- [Troubleshooting](troubleshooting.md)
- [Writing docs](writing-docs.md)
- [Glossary](glossary.md)
