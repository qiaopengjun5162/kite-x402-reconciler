# 任务二设计：kite-x402-reconciler（x402 结算对账与审计工具）

> 本文件记录指南要求的「先让 AI 阅读五个指定仓库 → 比较 → 提出新项目方案与验收标准」过程。
> 阅读日期：2026-11-17。方案确定：Rust CLI（纯后端，无需前端）。

## 落地状态（2026-11-17）

- ✅ **D1–D9 已完成**（数据模型 / 摄取 / 校验 / 聚合 / 报表 / CLI / 文档 / CI）
- ✅ 公开仓库：https://github.com/qiaopengjun5162/kite-x402-reconciler
- ✅ 首个 commit `00f40ca`，作者关联本人账号（github_login=qiaopengjun5162）
- ✅ tag `v0.1.0` 已推送
- ✅ tag `v0.2.0`（2026-11-20）：新增 `reconcile` 双子对账与 `--fail-on` 门禁，
  **76 测试**（51 单元 + 25 集成），clippy 零警告，MSRV 1.88 实跑通过
- ✅ 33 测试通过、fmt 干净、clippy 零警告、release 构建成功；六个子命令端到端验证通过
- ✅ 质量加固（2026-11-17 第二批）：新增 12 个 CLI 端到端集成测试（共 45 测试）、
  `docs/TESTING.md`、`demo.sh` 运行演示、README 配置与已知限制章节；全部门禁通过。
- ✅ 质量重构（2026-11-17 第三批）：修复按天聚合的时区口径（统一 UTC）、
  CI 增加 MSRV 1.88 真实验证 job、`report --format csv` 改为单一表头、
  `amount_atomic()` 去嵌套、`ValidationReport::from_issues` 收口计数、
  合并重复 hex 校验。共 49 测试，稳定版 + 1.88 双工具链验证通过。
- ⏳ **D10 / 活跃日**：这批工作仍在同一自然日 = 仍只算 1 个活跃日；
  后续需按指南把迭代分散到不同日期（见第六节），每个日期至少一次有效提交。
- ⏳ **任务二提交材料**：按指南每次提交需含仓库链接、本期新增 Commit、变更说明、
  测试结果、运行演示；待组织者公布第二期提交入口后提交。

## 一、五个指定仓库的阅读结论（设计依据）

| 仓库 | 语言 | star | 工程完整度 | 可借鉴点 | 不复制的原因 |
|---|---|---|---|---|---|
| `CarryWang/kite-x402-explorer` | TS | 30 | 高（docs/ARCHITECTURE.md、多份设计文档、Docker） | 文档组织、架构说明的写法 | 前端 Explorer，与本工具定位不同 |
| `BrKDDD/kite-x402-hono` | TS | 29 | 很高（中英 README、CONTRIBUTING、CHANGELOG、SECURITY、LICENSE、docs 周报） | 文档矩阵与 CHANGELOG 规范 | TS/Hono 栈，且是支付包装器 |
| `tianzeshi-study/kite-x402-axum` | Rust | 33 | 极高（workspace、367 测试、CI/audit、14k+ 行） | **SettleResponse 数据模型**、CAIP-2 network、原子单位金额、错误语义 | 它是任务一的贡献目标；本工具是独立品类（对账） |
| `Anyi-zheng/kite-agent-zk-attestation` | TS | 0 | 中（ZK 合约 + docs） | docs 分文件的组织方式 | ZK 方向跨度大 |
| `gokite-ai/kite_counter_dapp` | TS | 37 | 教学示例（含 dist/、node_modules） | —— | 只是 counter 教学合约 |

**从 `kite-x402-axum` 读到的真实结算数据结构**（`src/types.rs:102-129`，本工具直接对齐，保证 KiteAI 技术关联是真实的）：

```
SettleResponse {
  success: bool,
  errorReason?: String, error_message?: String,
  payer?: String,
  transaction: String,       // 结算交易哈希
  network: String,           // CAIP-2，如 eip155:2424
  amount?: String,           // 原子单位金额（字符串，不可用浮点）
}
```

## 二、新项目定位

- **目标用户**：在 Kite 链上跑 x402 付费服务的开发者 / 运营者。
- **使用场景**：facilitator 结算后会返回 `SettleResponse`（也被 base64 编码进 `PAYMENT-RESPONSE` header）。
  当服务有任何 HTTP 请求被计费后，需要回答三个问题：
  1. 这批结算记录**合法吗**（字段、network、金额、success 语义、重复）？
  2. 我**赚了多少**（按网络 / payer / 日期聚合，原子单位精确计算）？
  3. 有没有**重复结算 / 重放**（重复 transaction、重复 nonce）？
  现有 5 个仓库都在解决「怎么收钱」，**没有一个解决「收完之后怎么对账」**——这是真实缺口。

- **与 KiteAI 的技术关联（可核验）**：
  - 数据模型字段逐一对齐 Kite facilitator 的 `SettleResponse`；
  - 支持解码 x402 的 `PAYMENT-RESPONSE` base64 header；
  - `network` 按 CAIP-2 解析校验;
  - `amount` 按链上原子单位（u128）精确运算，不引入浮点误差。

## 三、功能清单

| 子命令 | 功能 |
|---|---|
| `validate` | 校验一批结算记录，输出 Error/Warning 问题清单 |
| `summary` | 按 network / payer / day 聚合笔数、成功失败、总金额 |
| `duplicates` | 检测重复 transaction 与重复 nonce（payer+nonce） |
| `report` | 导出 CSV / JSON 对账报表 |
| `decode` | 解码一个 `PAYMENT-RESPONSE` header 为结算记录 |
| `sample` | 生成示例账本（demo / 测试用） |

## 四、技术方案

- Rust 2021 edition，纯 std + `serde`/`serde_json`/`chrono`/`clap`/`thiserror`/`base64`/`anyhow`。
- 库 + CLI 双层：`src/lib.rs` 暴露可复用 API，`src/main.rs` 是 clap CLI。
- 金额一律 `u128` 原子单位；展示时才按 decimals 转字符串（不用浮点）。
- 输入支持 JSON 数组 / JSONL / 单个对象 / `-`（stdin）。

## 五、验收标准（Definition of Done）

1. `cargo test --workspace` 全绿；`cargo clippy --all-targets -- -D warnings` 零警告；`cargo fmt --check` 干净。
2. 六个子命令都能端到端跑通，有可复现示例。
3. README + docs/ARCHITECTURE.md + 使用文档 + `.env.example` + CHANGELOG.md + LICENSE 齐全。
4. 所有校验规则、边界（空输入、非法金额、CAIP-2 边界、重复）都有测试。
5. 公开仓库，commit 关联本人 GitHub 账号，原创代码（不复制五个仓库的实现）。

## 六、10 个活跃日拆分（每自然月 ≥10 个不同日期有效贡献）

| 日 | 内容 |
|---|---|
| D1 | 数据模型 `model.rs`（SettleResponse 对齐、CAIP-2、原子单位） + 单测 |
| D2 | `ingest.rs`（JSON/JSONL/stdin、PAYMENT-RESPONSE base64 解码） + 单测 |
| D3 | `validate.rs` 基础规则（success/network/amount/hash/payer） + 单测 |
| D4 | `validate.rs` 重复检测（transaction / nonce） + 边界单测 |
| D5 | `reconcile.rs` 聚合（network/payer/day、u128 精确金额） + 单测 |
| D6 | `report.rs`（CSV/JSON/表格、CSV 转义） + 单测 |
| D7 | `main.rs` CLI 六个子命令接线 + 集成测试 |
| D8 | 文档：README / ARCHITECTURE / USAGE / .env.example / CHANGELOG / LICENSE |
| D9 | CI workflow + 边界加固 + 回归测试 |
| D10 | 发仓库、打 tag、按反馈修复 |

> 实际提交日期会分散；指南只认「不同日期的有效提交」，一天多 commit 算一天。

## 七、v0.1.0 之后的迭代（持续维护）

发布后按「缺陷 → 收紧契约 → 补功能」的顺序推进，每个改动都带反例测试。

| 轮次 | 内容 | 类型 |
|---|---|---|
| R1 | `report --format json` 输出粘成两个 JSON 对象（不可解析）；`u128` 溢出 debug panic / release 静默回绕；README decode 示例是坏 base64 | 缺陷 |
| R2 | 按天归并改 UTC 归一；CSV 双表头改单表头 + `section` 列；`amount_atomic` 两层 `Option<Result>` 收成一层；CI 真跑 1.88 验 MSRV | 收紧契约 |
| R3 | `--fail-on none\|error\|warning` 门禁 + 退出码 `3`（避开 clap 的 `2`），`--strict` 降级为隐藏别名且只能收紧 | 功能 |
| R4 | 新增 `reconcile` 双子命令：本地台账 vs facilitator 台账按交易哈希配对，输出 `only_local` / `only_facilitator` / `amount_mismatch` | 功能 |

### R4 的设计取舍

此前所有命令都只审**一份**台账，回答的是「这份台账自洽吗」，而不是「它和链上一致吗」——后者才是赔钱的地方。名字叫 reconciler 却没有对账能力，是设计上的漏洞，R4 补的就是这块。

三条判断值得记下来：

1. **失败结算不参与配对**。失败的 `SettleResponse` 没有链上交易，拿它去对另一侧的「没有」会把每一次失败记成两条差异。真实台账里失败占多数，naive diff 的输出基本全是噪声。改为计入 `skipped_failed`，只报数量。
2. **「成功但没有交易哈希」是缺陷，不是噪声**。它声称成交了却无法识别，必须冒泡为 warning，否则会静默漏出对账范围。
3. **配对按输入顺序，多余的一份变成单边发现**。想做得更聪明（按金额配对）反而会把「同 hash 重复入账」这个真实问题藏起来。
