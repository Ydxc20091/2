# DbotX Trade Demo

一个最小可用的 Rust 示例，用于演示如何调用 DbotX 的 **Data** 与 **Trade** API：
- 通过 **Solana token mint (CA)** 搜索可用池子
- 自动选择流动性更好的池
- 用 API Key 提交一笔 **IOC 限价**买单

> 示例代码仅用于演示。生产环境请补齐签名/鉴权、风控与错误处理。

---

## Requirements

- Rust 1.72+（或更高）
- 已申请的 DbotX API Key（通过请求头 `X-API-KEY` 传入）

## Build

```bash
cargo build --release
