# NovaVei

NovaVei 是一个以本地数据边界为优先的桌面 AI 工作台，基于 Tauri 2、Rust、TypeScript 与嵌入式 Pi runtime 构建。

`NovaVei` 是项目、产品和 GitHub 仓库的正式名称。为避免破坏既有安装与便携版数据，Rust crate、bundle identifier 和现有可执行文件的内部兼容标识暂保留 `novavei-agent`；它们不是对外产品名称。

## 快速开始

环境要求：Node.js `24.18.0`、npm `11.16.0`、Rust `1.96.1`、Windows WebView2 Runtime，以及 Windows 上的 MSVC Build Tools。

```powershell
npm ci
npm run dev
```

常用命令：

```powershell
npm run typecheck
npm run lint
npm run format:check
npm run verify
npm run build
npm run pack:portable
npm run build:installer
```

`npm ci` 始终使用已提交的 `package-lock.json`。发布前仍应在受控 Windows 环境中完成真实 Provider/MCP、WebView、签名和安装包验证；本地构建不等同于发布验收。

## 仓库结构

```text
src/                 WebView UI 与 runtime bridge
src-tauri/           Rust 主机、IPC、存储与安全边界
scripts/             可公开的打包与发布检查脚本
.github/             GitHub Actions 与 Dependabot 配置
```

## 隐私与提交边界

本仓库不会提交依赖缓存、构建产物、便携版数据、SQLite 数据库、WebView profile、日志、`.env`、证书、密钥、令牌、用户配置、设计稿或本地测试/诊断材料。完整规则见 [`.gitignore`](./.gitignore)。

请勿将 API Key、Provider header、Cookie、MCP 凭据、签名证书或用户导出的诊断/聊天数据加入提交。若意外暴露过任何真实凭据，应立即在对应服务端轮换。

前端优先使用显式模块 import；`withGlobalTauri` 仅为受限迁移兼容层，不能作为新增功能的默认接入方式。

## GitHub 准备状态

- GitHub Actions 使用固定 SHA 的第三方 Action，并以最小 `contents: read` 权限运行。
- Dependabot 覆盖 npm、Cargo 与 GitHub Actions。
- 许可证为 [MIT](./LICENSE)。
- 目标仓库为 [zaixiaziyang/NovaVei](https://github.com/zaixiaziyang/NovaVei)。若远端已有初始 `LICENSE` 提交，先获取并合并其历史，再正常推送；不要以无保护的强推覆盖它。

```powershell
git remote add origin https://github.com/zaixiaziyang/NovaVei.git
git fetch origin
git merge --allow-unrelated-histories origin/main
git push -u origin main
```
