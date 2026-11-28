-- 样本 2：多级指针（CE 里写成 "game.exe"+0123456 -> 10 -> 18）
-- Zero 的地址解析支持同样的语法：模块名+偏移，逗号后面每一段都是一次解引用。
--
--   真实 CE 表里的写法        Zero 里的写法
--   game.exe+0123456          "game.exe+0123456"
--   -> 10 -> 18               "game.exe+0123456,0x10,0x18"
--
-- 下面这段用符号和 readPointer 演示同一条链，这样在练习靶场上也能跑。

-- 1) 自己铺一条三级链：node1 -> node2 -> value
local node1 = allocateMemory(4096)
local node2 = allocateMemory(4096)
local value = allocateMemory(4096)

writePointer(node1, node2)
writePointer(node2, value)
writeInteger(value, 4242, 4)
print(string.format("链头 %X -> %X -> %X", node1, node2, value))

-- 2) 顺着链走：每一跳都按目标进程的指针宽度解引用
local hop1 = readPointer(node1)
local hop2 = readPointer(hop1)
print(string.format("走了两跳，读到 %d", readInteger(hop2, 4)))

-- 3) 符号等价于 CE 的 registerSymbol：注册一次，之后 getAddress 会先查它
registerSymbol("player_hp", value)
print(string.format("符号 player_hp 解析到 %X", getSymbol("player_hp")))

-- 4) 真游戏里的模块写法（靶场没有 game.exe，所以放在注释里）：
--    local hp = getAddress("game.exe+0123456,0x10,0x18")
--    if hp then print(string.format("血量 %d", readInteger(hp, 4))) end

freeMemory(node1)
freeMemory(node2)
freeMemory(value)
print("链已释放。")
