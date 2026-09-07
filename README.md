# Local Hub · Tauri 2

面向 Windows 的本地服务控制中心。使用 React、TypeScript 和 Vite 构建界面，Rust 负责配置、进程、日志与托盘。原 WinForms 项目保留在上一级目录，两个项目独立构建。

## 功能

- 全部启动项、正在运行、自动启动、服务日志四个页面。
- 新增、编辑、复制、批量删除、搜索、排序、Ctrl / Shift 多选及右键菜单。
- 启动、停止、重启、依次自动启动；停止全部时取消待启动队列。
- 支持可执行程序、批处理、Python、PowerShell 脚本，以及 cmd、PowerShell 7、Windows PowerShell、WSL、Git Bash 命令。
- 后台捕获标准输出和错误流，支持中文、UTF-8 和带 BOM 的 UTF-16；日志可筛选、复制、清空、刷新和自动跟随。
- 将文件或文件夹拖入窗口创建启动项；支持打开工作目录、服务网址和日志中的 HTTP / HTTPS 链接。
- Windows 托盘、单实例、开机自启动、窗口位置记忆、关闭或最小化到托盘。

## 开发

需要 Windows 10 / 11、Node.js 22.12+、Rust 1.88+（MSVC 工具链）、Visual Studio C++ 构建工具及 Windows SDK。桌面运行需要 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。服务使用的 Python、Node.js、PowerShell 7、WSL 或 Git Bash 需另行安装。

从仓库根目录进入子项目：

```powershell
cd tauri-app
npm ci
npm run desktop:dev
```

本工作区的 Rust 依赖缓存保存在 `.cache/cargo`。继续使用该缓存时，在同一终端中先设置：

```powershell
$env:CARGO_HOME = Join-Path (Get-Location) '.cache\cargo'
```

`npm run dev` 只提供浏览器界面预览，管理本地服务需要通过 `npm run desktop:dev` 运行桌面应用。

## 构建

生成便携可执行文件：

```powershell
npm run desktop:build -- --no-bundle
```

产物：`src-tauri/target/release/LocalHubLauncher.exe`。该程序内置前端资源，不需要附带 `dist` 或安装 .NET；目标电脑需要 WebView2。

生成 Windows NSIS 安装包：

```powershell
npm run desktop:build
```

安装包输出到 `src-tauri/target/release/bundle/nsis/`，按当前用户安装。首次打包需要联网下载打包工具。配置存放目录必须可写。

## 从旧版本迁移

1. 退出旧版启动器，确认其服务已停止。
2. 备份旧目录中的 `config.json`，将配置复制到新版 `LocalHubLauncher.exe` 旁边。
3. 保留配置中使用的服务路径；相对路径以配置目录为基准。
4. 启动新版。版本 1 配置自动读取，兼容 PascalCase、大小写不同的字段及启动方式名称；保存时写为版本 2。
5. 如需 Windows 开机自启动，在新版设置中保存该选项，使注册路径指向新程序。

配置默认保存在可执行文件旁的 `config.json`。开发模式对应 `src-tauri/target/debug/config.json`，发布模式对应 `src-tauri/target/release/config.json`。需要使用独立工作空间时，可指定绝对路径：

```powershell
$env:LOCALHUB_CONFIG_DIR = Join-Path (Get-Location) '.cache\my-workspace'
npm run desktop:dev
```

无法解析或校验配置时，程序保留原文件并尝试创建 `config.json.broken-*` 备份，界面显示修复提示。修复后按 F5 重新读取。保存前会检查外部修改，刷新失败会保留当前配置；刷新涉及更换或删除正在运行的服务时，先停止对应进程。

## 启动示例

| 用途 | 启动方式 | 启动文件或命令 | 参数 / 工作目录 |
| --- | --- | --- | --- |
| Node 服务 | 单命令 · cmd | `node server.js` | 工作目录设为服务目录 |
| npm 项目 | 单命令 · cmd | `npm start` | 工作目录设为含 `package.json` 的目录 |
| Python 脚本 | Python 脚本 | `D:\services\api\server.py` | 参数例如 `--port 8000` |
| Python 虚拟环境 | 可执行程序 | `D:\services\api\.venv\Scripts\python.exe` | 参数为 `-u "D:\services\api\server.py"` |
| PowerShell 命令 | 单命令 · Windows PowerShell | `Write-Output "hello 中文"` | 直接填写完整表达式 |
| 批处理 | 批处理 | `D:\services\start.cmd` | 子进程与脚本一起受到管理 |

勾选“后台运行并捕获日志”时隐藏终端并捕获输出；取消后使用独立终端，日志页仅显示启动、停止等管理事件。Python 后台输出启用 UTF-8 和无缓冲模式。

## 验证

```powershell
npm run check
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run test:rust
```

已构建 Release 程序后，可运行真实桌面流程验证：

```powershell
npm run test:desktop
```

该脚本会打开编译后的应用，通过 WebView2 检查旧配置导入、增删改、多选排序、进程启停、日志、设置及刷新恢复。它在 `.cache/desktop-smoke-*` 中创建独立配置、浏览器缓存和截图，仅启动测试命令，完成后退出测试实例。运行前需关闭其他 Tauri 版实例。

仓库的 `.github/workflows/tauri.yml` 在 Windows 上执行前端测试、桌面构建、Rust 格式检查、Clippy 和 Rust 测试，并保存可执行文件。

## 项目结构与边界

```text
tauri-app/
  src/                    React 界面与桌面调用
  src-tauri/src/          Rust 配置、进程、日志与桌面集成
  src-tauri/icons/        应用图标
  src-tauri/capabilities/ 窗口权限
  public/                前端静态资源
  scripts/               桌面流程验证
```

Windows 进程在创建时加入独立 Job Object，因此脚本退出后仍可跟踪并停止它的 Windows 子进程。运行状态表示进程组是否存活，不代表 HTTP 服务已就绪；WSL 内部 Linux 进程不提供独立的进程树状态。

日志保存在内存中，退出后不保留。每个视图最多保留 4000 行，并有额外的字节容量限制；大量输出时实时事件会合并并跳过部分行，点击刷新可读取当前缓冲。当前以 Windows x64 为验证目标。
