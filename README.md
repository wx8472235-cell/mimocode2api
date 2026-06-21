# mimocode2api

mimocode2api 把小米 MiMo Code 内置的免费模型转换成 OpenAI 兼容 API,基于 Rust + axum。启动后,任何支持 OpenAI 接口的客户端都可以直接接入,无需 API Key,也无需小米账号或 Cookie。

[![CI](https://github.com/wx8472235-cell/mimocode2api/actions/workflows/ci.yml/badge.svg)](https://github.com/wx8472235-cell/mimocode2api/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/wx8472235-cell/mimocode2api?logo=github&label=release)](https://github.com/wx8472235-cell/mimocode2api/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

> 本项目通过逆向 MiMo Code 与上游的交互协议工作,仅供学习研究,不是官方用法。上游接口随时可能变更、加风控或失效,请勿用于商业用途或任何形式的滥用。「MiMo」商标归小米所有,本项目与小米官方无关。

- OpenAI 兼容的 `/v1/chat/completions`、`/v1/models`,支持流式与非流式
- 零配置:客户端标识与认证令牌在本地自动生成并续期,不校验 API Key
- 单文件 Rust 二进制,无运行时依赖;提供 Docker 镜像和多平台预编译二进制

## 运行

Docker(最省事):

```bash
docker run -d --name mimocode2api -p 8080:8080 -v mimocode2api-data:/data \
  ghcr.io/wx8472235-cell/mimocode2api:latest
```

或用 docker-compose:

```bash
git clone https://github.com/wx8472235-cell/mimocode2api.git
cd mimocode2api
docker compose up -d
```

也可以从 [Releases](https://github.com/wx8472235-cell/mimocode2api/releases) 下载对应平台的二进制直接运行,或从源码构建(需要 Rust 1.85+):

```bash
cargo run --release
```

服务默认监听 `8080`。

## 用法

base URL 为 `http://localhost:8080/v1`,模型名 `mimo-auto`。代理不校验 API Key,客户端填任意非空字符串即可。

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "mimo-auto",
    "messages": [{"role": "user", "content": "你好"}]
  }'
```

加 `"stream": true` 开启流式响应。

```python
from openai import OpenAI

client = OpenAI(base_url="http://localhost:8080/v1", api_key="anything")
resp = client.chat.completions.create(
    model="mimo-auto",
    messages=[{"role": "user", "content": "你好"}],
)
print(resp.choices[0].message.content)
```

端点:

- `POST /v1/chat/completions` — 聊天补全,支持流式与非流式
- `GET /v1/models` — 模型列表
- `GET /health` — 健康检查

## 配置

通过环境变量:

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `MIMOCODE_PORT` | `8080` | 监听端口 |
| `MIMOCODE_MODEL` | `mimo-auto` | `/v1/models` 返回的模型名 |
| `MIMOCODE_CLIENT_FILE` | `./.mimocode/client` | 客户端标识持久化路径(Docker 内为 `/data/client`) |
| `RUST_LOG` | `info,mimocode2api=debug` | 日志级别 |
| `MIMOCODE_HOST_PORT` | `8080` | docker-compose 映射到宿主机的端口 |

## 说明

- 上游只提供一个免费模型,`MIMOCODE_MODEL` 只影响显示名,不会切换实际模型。
- 代理不校验 API Key 且默认放行跨域,不要直接暴露到公网;如需公网访问,请自行在前面加鉴权。
- 工具调用、多模态等参数会原样转发,是否生效取决于上游支持。

## 开发

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

MIT,详见 [LICENSE](./LICENSE)。
