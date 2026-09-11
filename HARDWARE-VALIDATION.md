# 分体 L2CAP CoC 上板验证清单

这个分支把分体链路从 GATT 换成了 L2CAP CoC，主机侧测试全过但**一次都没上过板**。
合并前门槛是下面三个测试。测完删掉本文件。

用 `examples/use_rust/nrf52833_ble_split`，两块 nRF52833，各自一个 J-Link。
这个例子是为这次验证建的：引脚、flash 布局照实际分体板，SDC builder 两侧都声明了
subrating 支持——`nrf52840_ble_split` 只开了 rmk 的 `subrating` feature 却没开控制器
那一半，subrate 请求直接被 HCI 拒掉，睡眠窗口根本进不去，而那正是测试 1 要打的地方。

---

## 0. 环境准备

### 日志级别

`.cargo/config.toml` 里已经是 `DEFMT_LOG = "info"`。**测试 1 和测试 2 不要往上调**，
因为分体路径上每个按键都有 `debug!` 打点：

- `rmk/src/split/driver.rs:180` — `Sending message to peripheral`
- `rmk/src/split/peripheral.rs:258` — `Writing split message`

这些在连打时会造成 RTT 反压，足以掩盖或者伪造出测试 1 要找的现象。

真要调的话改完必须清一次，环境变量变化不会触发依赖重编：

```bash
cargo clean -p rmk --release
```

### RTT 陷阱

defmt-rtt 默认 BlockIfFull。**读端断开会让固件永久阻塞，表现为假死**。
所以：先起 `defmt-print`，再复位板子；测试期间不要关读端。

### 其他

- `embassy-time` 开了 `defmt-timestamp-uptime`，每行日志自带设备上行时间，测试 2 直接用它算。
- 这个例子开了 `subrating`，睡眠走的是 subrate 而不是 conn param。
- J-Link 用 `JLinkExe` / `JLinkRTTLogger`，不要 `pkill JLinkGDBServerExe`。
- macOS 没有 `timeout`，需要限时用 `perl -e 'alarm shift; exec @ARGV' 30 <cmd>`。

### 烧录

```bash
cd examples/use_rust/nrf52833_ble_split
cargo build --release
./run.sh target/thumbv7em-none-eabihf/release/central      # 接主手那块
./run.sh target/thumbv7em-none-eabihf/release/peripheral   # 接副手那块
```

两块板各接一个 J-Link 时用 `SelectEmuBySN` 指定，别让它自己挑。

### 关键日志对照

本分支（CoC）：

| 日志 | 位置 | 含义 |
| --- | --- | --- |
| `Connected to peripheral {n}` | 中心 | ACL 链路建立 |
| `Split channel open, mtu 21` | 两侧 | 通道可用，会话开始 |
| `Opening the split channel timed out` | 中心 | 建链失败，5s |
| `Central connected but never opened the split channel` | 外设 | 建链失败，10s |
| `Split channel send timed out waiting for credits` | 两侧 | **测试 1 的失败信号** |
| `Split channel send/receive error` | 两侧 | 通道层错误 |
| `[split] subrating updated: subrate ...` | 外设 | 睡眠参数已落地 |
| `Connection lost` / `Disconnected from central` | 中心 / 外设 | 断链 |

基线（main，GATT）在测试 2 里对照用：

`Connected to peripheral {n}` → `Services found` → `Message to central found` → `Subscribing notifications`

建基线工作树。这个例子在 main 上不存在，所以连目录一起拷过去——例子的 `rmk`
是相对路径依赖，拷过去就自动编到那个工作树的 rmk：

```bash
# 从本工作树里跑，所以是 ../coc-baseline，不是 ../rmk.worktrees/coc-baseline
git worktree add --detach ../coc-baseline HEAD
rsync -a --exclude target examples/use_rust/nrf52833_ble_split \
  ../coc-baseline/examples/use_rust/
```

基线已经建好并量过了，体积对比见下表。

---

## 进展

- 2026-09-11：`nrf52833_ble_split` 双板上板，基本通路通过——通道建立、两个方向的
  消息都通、正常打字无异常。这只说明 CoC 路径能工作，三个门槛测试一个都还没做。
- 2026-09-11：收发改成并发，修掉死锁环和 `receive()` 取消不安全两个缺陷（见测试 1）。
  523 个行为测试全过，三行 feature 组合 `-D warnings` 干净。**上板结果作废，要重测。**

---

## 1. 唤醒瞬间连打

**这是唯一一个我明确知道可能比 GATT 差的场景，先做这个。**

改动把 `notify` 的"发了不管"换成了 credit 流控。初始 credit 16 个，一条消息正好一帧，
按下加抬起两帧，所以约 8 次击键就会用掉一个完整窗口。中心在收满 8 帧时补发
（`CreditFlowPolicy::MinThreshold(8)`）。如果补发赶不上，外设的 `write` 会阻塞，
卡满 10 秒后 `SEND_TIMEOUT` 触发、拆链重连。

而睡眠期 subrate 拉到 100，锚点间隔约 750ms，credit 往返被同步拉长。
**最危险的窗口就是按键唤醒之后、subrate 还没降回来的那几百毫秒。**

### 步骤

1. 日志保持 `info`，重编重烧两块板。
2. 两侧 `defmt-print` 都起好。
3. 让键盘进睡眠。两条路，任选：
   - 空闲 60 秒（例子的 `keyboard.toml` 里 `split_central_sleep_timeout_seconds = 60`；
     这个值默认是 0，而 0 表示睡眠管理整个关掉，`nrf52840_ble_split` 就是这种——
     它连睡都睡不着，更不可能复现测试 1）；
   - 让主机 HID suspend（把电脑睡了），更快。
4. 确认外设打出 `[split] subrating updated: subrate 100` 或 `subrate 30`。没有这行就是没睡着，重来。
5. **在副手上尽可能快地连打 20 次以上**，要真的快，别一秒一下。
6. 重复 10 轮。

### 判定

- **通过**：从不出现 `Split channel send timed out waiting for credits`，
  按键全部到达主机，允许有一次短暂延迟尖峰。
- **失败**：出现上面那行，或者出现 `Connection lost` 后紧跟重连。

### 失败了怎么办

按代价从低到高：

1. `CreditFlowPolicy::MinThreshold` 调大（`rmk/src/split/ble/mod.rs`），比如 12，更早补发。
2. `L2capChannelConfig.initial_credits` 显式调大，把窗口本身加宽。
   `None` 时取 `min(L2CAP_RX_QUEUE_SIZE, 包池容量)`：前者 16（`rmk/Cargo.toml`
   显式开的 `l2cap-rx-queue-size-16`），后者 32，所以卡在 16。真要加宽得先把
   feature 换成 `l2cap-rx-queue-size-32`，每个通道槽位也会跟着变大。
3. **不要**去掉 `SEND_TIMEOUT`。它是打破下面那个死锁环的唯一机制。

### 已修：死锁环 + `receive()` 取消不安全

原来两侧的循环都是 `select(read(), 待发事件)`，select 一解析就丢掉另一臂。这带来两个
缺陷，都已经改掉。

**死锁环。** credit 是接收方消费后才补发的，而 `send()` 在飞的时候 `read()` 不被 poll：

1. 外设 A 方向（外设→中心）credit 用尽，卡在 `send`。
2. 外设因此停止 `read`，不再消费 B 方向，不再给中心补 B 的 credit。
3. 中心继续发 B，外设不收，堆满外设那 16 深的入站队列，B 的 credit 归零。
4. 中心下一次 `send` 也卡住。环闭合，只有 `SEND_TIMEOUT`（10s）能打破，代价是整次重连。

**`receive()` 取消不安全。** `channel_manager.rs:1124-1137` 的 `flow_control()` 先
`process()` 决定补发 N 个，await 把 `LeCreditFlowInd` 发出去，**之后**才
`confirm_granted`；而 `receive()` 是先把 SDU 出队、拷给调用者，再 await 这个补发。
所以取消 `receive()` 有两种后果：拷贝之后、补发之前取消，那条消息被静默吞掉；补发已
上天、`confirm_granted` 之前取消，下次 `receive()` 会**再补一次 N**，对端拿到双倍
credit，一超发就撞上 `channel_manager.rs:536`，关掉整条 ACL 链路。

**改法。** 收发改成并发：`receive()` 只有一个调用点，是一个永远不和别的东西赛跑的读
循环（`run_peripheral_manager` / `run_split_peripheral_session`，各自 `select` 两个
独立的循环）。通道用 `L2capChannel::split()` 分成两半，两个循环各持一半，不再共享
`&mut`。`send()` 取消是安全的（SDU 只有一帧，`CreditGrant::Drop` 把没用掉的 credit
还回去），所以发送那一侧留在 select 里没问题。

外设的读路径原来会在收到 `ConnectionStatus` 时直接回一条 `BatteryStatus`，现在改成
发布 `BatteryStatusEvent`，由发送循环从它本来就订阅的那条路转发出去——读路径因此完全
不需要 writer。

`dfu_split` 不受影响：`src/lib.rs:41` 有 `compile_error!` 明确禁止它和 BLE 共存，
DFU 透传只走有线分体，那条路保留原来的串行循环。

串口分体和 BLE 分体在任何一个构建里都不共存——`pub mod serial` 本身就是
`#[cfg(not(feature = "_ble"))]`，`PeripheralManager` 的唯一构造点在那里面。所以两种
循环形态都按 cfg 分开，每个构建只编译自己那一份，不再各自拖着对方的死代码。

`SEND_TIMEOUT` 保留，但它现在是纯粹的死链兜底，不再是唯一的破环机制。

---

## 2. 重连耗时对照

理论收益最大的地方：GATT 要五个串行 ATT 事务（MTU 交换、服务发现、两次特征发现、
CCCD 订阅写），CoC 只要一次 signalling 往返。这条一测就知道是真是假。

### 步骤

1. 日志保持 `info`，两边都重编。
2. 中心侧 `defmt-print` 存盘：`./run.sh ... | tee coc-reconnect.log`。
3. 复位副手（J-Link 里 `r` + `g`，或者直接按复位键），等它重连上。
4. 重复 20 次，每次间隔 10 秒以上，别让它们撞在一起。
5. 从日志里取每次的时间差：
   - 本分支：`Connected to peripheral` → `Split channel open`
   - 基线：`Connected to peripheral` → `Subscribing notifications`
6. 在 `coc-baseline` 工作树上重烧，重复一遍。

### 判定

- **通过**：CoC 的中位数不高于 GATT，且没有超过 1 秒的离群值。
- **失败**：出现 `Opening the split channel timed out`，或者有长尾比 GATT 更差。

这条如果只是持平而不是更好，不构成放弃理由——迁移的实测收益本来就是体积，
重连更快只是附带的。但如果**更差**，那就得查为什么。

### 体积收益（已实测，`nrf52833_ble_split`，stable，release）

| | flash | RAM |
| --- | --- | --- |
| 中心 GATT | 442284 | 65132 |
| 中心 CoC | 436320 | 58076 |
| | **−5964 (−1.3%)** | **−7056 (−10.8%)** |
| 外设 GATT | 283400 | 38708 |
| 外设 CoC | 262724 | 37100 |
| | **−20676 (−7.3%)** | **−1608 (−4.2%)** |

收发并发本身的代价：中心 +192 字节 flash / +32 RAM，外设 +220 / +232。

中心省的 RAM 全在 embassy 主任务的 future 里（45336 → 38248 字节）：GATT 客户端
状态机没了，`L2CAP_CHANNELS_MAX` 也从 `CONNECTIONS_MAX * 4` 降到 1。
外设省得更多的是 flash，因为整个 GATT server 和服务定义都不用了。

---

## 3. 挂机

前两条过了才值得开始，可以边日常使用边收。

### 步骤

1. 日志保持 `info`，或降到 `warn`，两边重编重烧。
2. 正常用，至少三天。`defmt-print` 全程存盘。
3. 统计 `Connection lost`、`Disconnected from central`、
   `Split channel send/receive error` 的出现次数。
4. 有 GATT 版的历史体感可以对照；没有的话，这条就当绝对基线。

### 判定

- **通过**：断连次数不明显高于 GATT 版的日常体感，且没有需要手动复位才能恢复的情况。
- **失败**：出现死到重启才恢复，或者断连明显变频。

---

## 通过之后

不要直接翻默认值。`trouble` 的 `channel_manager` 里程数远低于 `gatt.rs`，
先加 feature flag 让 CoC 和 GATT 共存一个版本周期，默认仍走 GATT，
收到现场数据再翻。共存的代价是一点 flash，用来对冲这个消不掉的风险。

## 已知的、和本次测试无关的欠账

- 分体链路仍是明文，没有加密和绑定。
- `SplitMessage` 没有版本握手。中间夹着 cfg 门控变体，两端 feature 不一致时
  postcard 的变体下标会错位、静默解成另一个变体。两半固件版本偏移时会踩到。
