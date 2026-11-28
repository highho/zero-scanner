-- 样本 3：自己分配内存、写一段代码、用定时器观察
-- CE 的 alloc / createthread 在这里对应 allocateMemory 与 createTimer。

local block = allocateMemory(4096)
if not block then
  print("分配失败：没有附加进程。")
  return
end
print(string.format("分配到的内存 %X", block))

-- 在目标进程里写一个计数器，脚本结束时这块内存会被释放
writeInteger(block, 100, 4)
print(string.format("写入 %d，读回 %d", 100, readInteger(block, 4)))

-- 定时器在脚本主体结束后按间隔回调。脚本结束前最多跑 100 次或 2 秒，
-- 所以这里只报第一次，避免刷屏。
local ticks = 0
createTimer(20, function()
  ticks = ticks + 1
  if ticks == 1 then
    print("定时器开始回调（每秒 50 次，脚本结束即停）")
  end
end)
print("主体结束，定时器开始。")
