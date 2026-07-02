# Data Directory

`data/` 用于本地 report server 和 demo 流程生成运行数据。源码仓库只应保留本说明、`.gitignore` 和 `.gitkeep`。

不要提交 `*.sqlite`、`*.sqlite-wal`、`*.sqlite-shm`、`*.log` 或 `*.pid`。需要可复现示例数据时，优先放入 `scripts/` 的 seed 文件或 `tests/fixtures/`。
