# NovaVei

NovaVei 是一个以本地数据边界为优先的桌面 AI 工作台，基于 Tauri 2、Rust、TypeScript 与嵌入式 Pi runtime 构建。主界面把会话、项目、运行状态和本机工具收在一个桌面窗口里，让敏感数据留在本地。

## 主要能力

- 会话与项目侧栏：置顶、归档、重命名、复制、删除、项目文件夹与搜索。
- Composer：附件、权限选择、模型与推理强度、会话目标。
- 右侧 dock：运行、文件、Git、对比、浏览器；其中浏览器是原生子 WebView。
- 本机服务：Skills、MCP、Memory、Cron、知识库与技能目录。
- 便携模式：支持 `installed` / `portable` 之间切换，变更在下次完整重启后生效。
- 安全边界：凭据、密钥、缓存和本地数据保留在 native 层；浏览器预览不会伪造原生能力。

## 快速开始

环境要求：Node.js `24.18.0`、npm `11.16.0`、Rust `1.96.1`、Windows WebView2 Runtime，以及 Windows 上的 MSVC Build Tools。

```powershell
npm ci
npm run dev
```

浏览器预览：

```powershell
npm run dev:web
```

常用命令：

```powershell
npm run typecheck
npm run lint
npm run format:check
npm run verify
npm run build
npm run build:installer
npm run pack:portable
npm run checklist:release-smoke
```

`npm run verify` 会串起类型检查、lint、IPC capability audit、`npm run test:ci-supply-chain`、格式检查和构建。`npm ci` 始终使用已提交的 `package-lock.json`。

`npm run pack:portable` 会先做便携打包审计，再执行无 bundle 构建并复制便携包。`npm run dev:web` 只启动浏览器预览，不提供原生浏览器、文件系统、MCP、便携存储或其他 Tauri 能力。发布前仍应在受控 Windows 环境中完成真实 Provider/MCP、WebView、签名和安装包验证；本地构建不等于发布验收。

## 验证与发布

- `npm run audit:ipc-capabilities`：检查渲染层和 native 能力边界。
- `npm run test:ci-supply-chain`：静态检查锁文件、Dependabot 和 Actions 安全约束。
- `npm run audit:portable-packaging`：检查便携包是否只包含预期文件。
- `npm run checklist:release-smoke`：打印需要真人、真实 provider 或真实 WebView 的发布验收清单。

## 仓库结构

```text
src/                 WebView UI 与 runtime bridge
src-tauri/           Rust 主机、IPC、存储与安全边界
scripts/             打包、审计与发布检查脚本
.github/             GitHub Actions 与 Dependabot
```

## 隐私与提交边界

前端优先使用显式模块 import；`withGlobalTauri` 仅为受限迁移兼容层，不能作为新增功能的默认接入方式。

## GitHub 准备状态

- GitHub Actions 使用固定 SHA 的第三方 Action，并以最小 `contents: read` 权限运行。
- Dependabot 覆盖 npm、Cargo 与 GitHub Actions，每周分组更新。
- 我们禁止在不说明过期原因和修复计划的情况下忽略 npm advisory（advisory ignore 过期）。
- 许可证为 [MIT](./LICENSE)。
- 依赖安全工作流位于 [`.github/workflows/dependency-security.yml`](./.github/workflows/dependency-security.yml)：它在每个 PR、每周计划任务和手动触发时运行；检出步骤不持久化仓库凭据。该工作流需要联网以查询 npm 与 RustSec 漏洞数据库。
