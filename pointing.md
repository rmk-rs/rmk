# 指点设备架构重构方案

## 背景

对比 ZMK 指点设备实现后的结论:RMK 的底子(驱动进主线、事件模型、模式实现、自动鼠标层)已普遍优于 ZMK 核心,当前的差距不在功能数量,而在四个架构问题上。本方案只处理架构,不做外围功能;所有外围功能(16-bit 报文、高分辨率滚动、Rynk 端点、手势解析等)在重构后变成架构挂点上的常规开发,见文末挂点表。

**范围界定**:

- 算架构的:事件模型的分类学、处理管线的形状、配置的生命周期、内部表示与线格式的关系。这四件事决定"以后每加一个功能要花多大代价"。
- 算外围、本方案明确不做的:16-bit descriptor、高分辨率滚动、TOML 语法糖、Rynk 端点、IQS5xx 手势位解析、加速曲线、新驱动。

## 现状的四个架构病灶

1. **离散输入没有归宿**。`PointingEvent` 只有轴([input.rs](rmk/src/event/input.rs));IQS5xx 读到手势直接丢弃;Caret 模式绕过整个 action 管线直发裸 HID 报告([pointing.rs](rmk/src/input_device/pointing.rs) 的 `tap_key`,代码注释自己承认丢修饰键);`Axis` 枚举定义了 Z/H/V 但 processor 只消费 X/Y。
2. **模式是硬编码的原语组合**。Sniper 就是 Cursor 换参数;Scroll = 轴重映射 + 缩放;invert/swap 同一概念存在于**三层**(传感器配置、processor 配置、每个模式配置各一份);想加"加速度"没有位置放——要么塞进全部四个模式,要么发明第五个模式。
3. **配置被冻结在编译期**。模式表是 codegen 常量 + 手写 controller;`PointingSetCpiEvent` 改了不存;而 keymap/BehaviorConfig 早已确立"TOML 默认 → storage 覆盖 → 运行期修改 → 持久化"的运行时实体模式,指点设备游离在外。
4. **内部管线绑死外部线格式**。`Report::MouseReport(usbd_hid::MouseReport)` 让 i16 的内部事件在组装点被迫截断成 i8;descriptor(线格式)和内部表示是同一个类型,改任何一边都牵动另一边。

---

## 决策 A:输入分类学 —— 一条公理管住所有输入

**决策**:整个输入系统只有两条通道,规则一句话——**能绑定 action 的离散输入,一律走 `KeyboardEvent`、由 keymap 解析;驱动指针的连续量,一律走 `PointingEvent`、由 processor 解析**。旋钮已经是这个模式(连续旋转 → 离散方向 → keymap),现在把它升格为公理。

**否决的方案**:给 `PointingEvent` 加 button/gesture 字段。那会造出第二条离散管线,绕过 keymap——按钮不能 remap、不能参与 tap-hold/combo/fork,等于在架构里埋一个永久的二等公民。

**步骤**:

1. rmk-types 定义 `PointingGesture` 枚举(Tap/TwoFingerTap/PressHold/Swipe×4/Zoom×2,离散触发的全集,也是未来 host 展示的单一来源)。
2. `KeyboardEventPos` 加第三变体 `Gesture(GesturePos { device_id, gesture })`,与 `RotaryEncoder` 平级;定义脉冲(tap 类,驱动合成按下+抬起)与保持(press-hold,真实按下/抬起对)两种语义。
3. keymap 加 gesture action 表(结构完全镜像 `encoder_map`),keyboard.rs 分发处与 encoder 同构处理。
4. **Caret 迁移**:四个方向就是四个离散触发——route 到 gesture 槽,经 keymap 解析成 action。删掉 `tap_key` 裸报告 hack,修饰键、remap、绑宏全部自动修复。`CaretConfig` 里的四个 keycode 字段消失,只留阈值参数。
5. `PointingProcessor` 补上 V/H 轴的消费路径(原生滚动直通 wheel/pan),并在模块文档里写死轴语义表:**Rel X/Y=运动,Rel V/H=原生滚动,Abs=位置(joystick 类,须先转 Rel),Z=保留**。
6. 驱动契约:一个设备可以同时是连续源和离散源——宏已支持 wrapper enum 多事件发布(`MultiDeviceEvent` 模式),IQS5xx 后续接手势时零宏改动。

**产出/不变式**:*不存在从驱动直达 HID 的路径*;任何离散输入天然获得完整行为栈和 remap 能力;"接通一个新触摸板的手势"从架构问题降级为"解析几个位段"的驱动细活。

---

## 决策 B:管线归一 —— 从"模式枚举"到"固定管线 + 参数档"

**决策**:把四个模式拆回它们本来的样子——同一条管线的不同参数。管线形状固定为三段:

```
transform(invert/swap) → scale(mul/div + 余数累积,预留 accel 槽) → route(→光标 | →滚轮 | →离散触发)
```

一组参数叫一个 **`PointingProfile`**;按 (device_id, layer) 选档。"Cursor/Scroll/Sniper/Caret" 从代码里的 enum 变体退化为**预设的参数档名**。层切换内置:processor 自己订阅 `LayerChangeEvent` 查档,换档时统一重置 accumulator(现在重置时机散落各处)。

**invert/swap 三层收敛为两层**,各有明确职责:传感器安装方向(硬件事实,编译期,留在驱动配置)+ 用户偏好(运行期,进 profile)。删掉每模式一份的第三层。

**否决的方案**:ZMK 式任意 processor 链。它的通用性是为"生态贡献者各自发布 processor 模块"设计的,RMK 是 monorepo 用不上;变长链让 storage/wire 类型复杂化(MaxSize 不定),解释执行也不符合 explicit-over-clever。固定形状 + 正交槽位覆盖全部真实场景,且参数档是**定长纯数据**——这正是决策 C 的前提。

**关键洞察**:模式-as-enum 是控制逻辑,参数-as-数据才是配置。"运行时可配"在架构上就等价于这次重构——不做 B,决策 C 就只能序列化一个 enum,加速度落地那天还得改一次协议。

**步骤**:

1. rmk-types 定义 `PointingProfile`(定长、`Serialize/Deserialize/MaxSize`),字段即三段管线的参数;accel 槽位**本版不加字段**(不做投机预留,协议 pre-release,落地时再加)。
2. `PointingProcessor` 重构为管线执行器:现有 `MotionAccumulator` 逻辑原样归入 scale 段,route 段吸收 Caret 阈值逻辑并输出到 A 的 gesture 槽。
3. 迁移三层 invert/swap:rmk-config 对旧字段给出明确报错+迁移提示(pre-1.0,breaking 可接受,但报错信息要指路)。
4. `PointingProcessorEvent` 载荷从 `PointingMode` 改为 `PointingProfile` 覆盖——手写 controller 的高级接口保留且变得更强。

**产出/不变式**:*新增一种指针能力 = 在管线上加一个槽位或参数,永不新增模式变体*;所有可调参数收敛进一个可序列化结构。

---

## 决策 C:配置生命周期 —— 指点配置成为"运行时实体"

**决策**:让 profile 表 + CPI 加入 keymap/BehaviorConfig 已经验证过的生命周期,不发明新机制:

```
TOML 默认值 → codegen 默认表 → boot 时 storage 覆盖 → 运行期整份替换事件 → FlashOperationMessage 持久化
```

同时把 **device_id 确立为贯穿主键**:事件、配置、存储、未来协议都用它索引设备。

**步骤**:

1. [storage](rmk/src/storage/mod.rs) 三元组(`FlashOperationMessage`/`StorageKey`/`StorageData`)各加 `PointingConfig { device_idx, … }`,整份存取(理由同 `BehaviorConfig` 注释:一条消息优于六次读改写)。
2. boot 加载走 BehaviorConfig 同路径——构造 processor 前读好、覆盖默认、传入构造函数。**刻意不用 pub/sub 传初始值**:订阅者晚于发布者的启动时序问题不值得引入。
3. 运行期更新:`PointingConfigEvent`(device_id + 整份 profile 表)→ processor 整份替换;写路径同时发 storage 消息。
4. `PointingSetCpiEvent` 处理补持久化,归入同一条写路径。

**产出/不变式**:*任何来源(TOML/Rynk/未来 Vial)的配置修改走同一条写路径*。之后 Rynk 的 Get/Set 端点纯粹是"接一个 handler 调这条路径"。

---

## 决策 D:报告层解耦 —— 内部全精度,线格式归传输层

**决策**:内部管线端到端 i16(事件本来就是 i16),`Report::MouseReport` 换成 RMK 自有结构;USB/BLE 传输层各自决定 descriptor 和序列化位宽。**本次线格式保持 i8 不变**——解耦本身是目标,升 16-bit 只是解耦后传输层的一个独立小 PR。

同时把**指针状态所有权契约**写进模块文档,替代引入新机制:按钮的单一事实来源 = `keymap.mouse_buttons`(鼠标键路径和 sensor 路径已共享);轴增量允许多生产者并存(相对增量天然可加,mouse keys 与传感器交错无害)。**不做聚合器**——没有它解决的真实 bug,属投机加固。

**步骤**:

1. [hid.rs](rmk/src/hid.rs) 定义自有 `MouseReport`(i16),替换 `Report` 枚举载荷。
2. 三个组装点适配(pointing.rs 删 clamp、mouse.rs 拓宽、joystick 同理)。
3. 序列化收口到传输层:[usb/mod.rs](rmk/src/usb/mod.rs) 的 `write_composite` 与 ble_server 各自负责 i16→线格式(当前 i8)的窄化,窄化处集中且被注释标记。
4. 所有权契约写入 pointing.rs / mouse.rs 模块文档。

**产出/不变式**:*改线格式只动传输层*。16-bit、hi-res scroll feature report 都变成传输层的局部改动。

---

## 依赖与顺序

```
D(报告解耦)──独立,最先做(纯机械,风险最低,先清场)
A(分类学)────独立,与 D 并行
B(管线归一)──依赖 A(Caret 的 route→gesture 槽)
C(生命周期)──依赖 B(Profile 是被管理的实体)
```

建议 4 个 PR:D → A → B → C。每个 PR 结束时固件行为对用户**完全等价**(D、A、B 都是等价重构 + 补洞,C 加的是尚无写入者的机制)——这保证任何一步可以独立合并、独立回滚。

## 外围功能的挂点(重构后各自变成多小的活)

| 推迟的功能 | 挂点 | 重构后的工作量 |
|---|---|---|
| 16-bit 报文 | D 的传输层 | 改 descriptor + 删窄化,一个小 PR |
| 高分辨率滚动 | D 的传输层 + B 的 scale 段读 multiplier | 局部 PR,不再牵动内部类型 |
| TOML `layer_mode` 语法 | B 的 profile 默认表 codegen | 纯 rmk-config/rmk-macro 表面活 |
| Rynk Get/SetPointingConfig | C 的写路径 + `PointingProfile` 已是 wire type | 接 handler + 快照/TS 例行更新 |
| IQS5xx 手势/双指滚动 | A 的 gesture 通道 + V/H 直通 | 驱动内解析位段,纯函数可单测 |
| 加速曲线 | B 的 scale 槽位 | 加参数字段 + 一段定点计算 |
| 新传感器驱动 | `PointingDriver` trait 不动 | 与今天相同 |

一句话总结:四个决策分别回答"离散输入归谁管"(keymap)、"能力如何组合"(固定管线+参数档)、"配置活在哪里"(运行时实体)、"精度在哪截断"(传输层)——答案确定后,所有外围功能都从"架构问题"降级为"挂点上的常规开发"。
