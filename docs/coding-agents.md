# Coding-agent compatibility

Tablinq's local gateway is OpenAI-compatible in shape, but coding-agent use is currently unsupported and not recommended.

We attempted a coding-agent integration and found that the browser-backed transport is not dependable for long repository-changing workflows. The ChatGPT UI is a presentation surface, not a lossless structured tool channel. Exact JSON, source code, shell commands, repeated writes, and verification loops can be changed, delayed, or rejected before a client can use them.

That limitation is architectural. Prompt wording and small compatibility patches cannot make a browser-rendered response equivalent to a native structured tool-call provider. The combination of one attached browser session, serialized work, final-only streaming, changing UI behavior, and limited subagent support makes failures increasingly likely as a coding task grows.

## Recommended use

Prefer Tablinq for:

- read-only repository questions and code discussion;
- bounded, user-supervised actions;
- simple scripts that can tolerate a failed or delayed request;
- research, file analysis, and artifact retrieval through the local gateway.

For autonomous coding work across a large repository, use a provider with native structured tool calling instead.

## If you experiment anyway

Keep the integration deliberately small:

- use `http://127.0.0.1:3847/v1` and the local gateway API key from `config.toml`;
- keep `max_parallel_requests = 1` and `max_parallel_branches = 1`;
- leave subagents disabled;
- treat `stream = true` as final-only SSE, not live token streaming;
- persist `request_id` and inspect request status before retrying after a timeout or disconnect;
- do not assume that an OpenAI-compatible response shape provides native-provider behavior.

See [provider-mode.md](provider-mode.md) for the supported gateway routes, files, artifacts, conversation inspection, and recovery behavior.
