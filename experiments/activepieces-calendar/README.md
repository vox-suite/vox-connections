# Calendar piece experiment

See [the runtime decision and evidence](../../docs/activepieces-runtime-evaluation.md).

```sh
npm ci --ignore-scripts
npm test
```

This uses actual pinned published Activepieces action functions and mocked Google HTTPS with networking disabled. The authority port is synthetic and in memory; this is not a production integration, distributed approval implementation or live provider certification.
