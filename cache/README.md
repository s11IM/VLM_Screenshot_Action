# cache —— 可随时整体清空

这个文件夹里的所有内容都是可再生成的临时产物或过期文件，**整个 `cache/` 随时可以直接删除**，不会影响项目运行（下次 dev/build 会自动重建需要的部分）。

- `vite/` —— Vite 的预构建缓存（原 `node_modules/.vite`，删除后首次启动稍慢）
- `logs/` —— 手动归档进来的旧日志，放这里即表示允许被清理

日常无需手动清空：`npm run dev` / `build` 前会自动执行 `scripts/cleanup-cache.ps1`，清除超过 24 小时的缓存内容。
