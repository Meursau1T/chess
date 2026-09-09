# Chess TUI

一个以中文着法为主的终端中国象棋。棋盘参考 [techkang/xiangqi](https://github.com/techkang/xiangqi)，使用中文文字渲染棋子，以 `·` 表示棋盘交叉点和合法落点。

## 运行

```bash
cargo run --release
```

程序会依次查找 `--engine` 参数、`PIKAFISH_PATH` 环境变量、项目下的 `engine/pikafish`，最后查找 `PATH` 中的 `pikafish`。找不到时仍可运行，并自动使用内置轻量引擎。

安装官方 Pikafish 后再启动程序：

```bash
./scripts/install-pikafish.sh
cargo run --release
```

也可以使用已有引擎：

```bash
cargo run --release -- --engine /path/to/pikafish --think-ms 1500 --difficulty 12
```

## 着法和命令

直接输入中文着法并按 Enter，例如 `马二进三`、`炮二平五`、`车九进一`。同时接受繁体字、阿拉伯数字和 ICCS 坐标，例如 `馬二進三`、`马2进3`、`h0g2`。

可用命令包括 `新局`、`悔棋`、`提示`、`分析`、`停止`、`翻转`、`执红`、`执黑`、`双人`、`难度 1` 至 `难度 20`、`emoji`、`中文棋子`、`帮助` 和 `退出`。输入 `/` 会打开斜杠命令列表，可用上下方向键选择并按 Enter 执行。输入 `难度` 可查看当前等级，20 级是不限制棋力。

输入区是默认焦点。按 Tab 切到棋盘后，可用方向键或 `hjkl` 移动光标，用空格选子。合法的空落点显示为亮色 `·`，可吃的棋子使用高亮背景。黑方路数使用阿拉伯数字，红方路数使用中文数字；翻转棋盘后标签会跟随双方位置。再次按 Tab 回到输入区。

## 棋子样式

默认使用中文棋子。若想临时切回 Unicode 象棋符号，可使用 `--emoji` 启动或在程序内输入 `emoji`。

## Pikafish

Pikafish 通过标准 UCI 子进程接入。程序会在开局及每步棋后自动评估当前局面，并把评分保存在棋谱旁。评分统一采用红方视角，正数表示红方占优，负数表示黑方占优，`+1.00` 约等于红方多一兵；`Δ` 显示相对前一局面的变化。后台评估不会阻止玩家继续走棋。提示和分析时还会展示搜索深度、用时、NPS 和三条主要变化，并把引擎坐标着法转换为中文棋谱。

难度设计参考 [xiangqiai.com](https://xiangqiai.com/#/) 的 Pikafish `Skill Level`。参考站实际提供 0 到 20 级，20 级为不限棋力；本项目按 1 到 20 级呈现。使用 Pikafish 且难度低于 20 级时，程序会分析四个候选着法，在 `难度 + 1` 层记录候选，再按 Pikafish 的加权随机规则选择着法；等级越低，记录候选越早，选择次优着法的偏差越大。20 级始终采用引擎首选。`--think-ms` 仍单独控制每步思考时间，难度等级没有固定 Elo。

Pikafish 使用 GPLv3。本项目代码使用 MIT，安装脚本从官方仓库取得 Pikafish 的源码、许可证和神经网络，不把引擎二进制纳入本项目。

## 检查

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
