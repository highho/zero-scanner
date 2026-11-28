-- 样本 1：特征码定位 + 可还原的字节替换
-- 真实 CE 表里这一段一般写成：
--   aobscanmodule(health, game.exe, 48 8B 05 ?? ?? ?? ??)
--   health:
--     db 90 90 90
-- Zero 里用 AOBScan 找地址，用 writeBytes 写，用读回来的原值还原。

-- 1) 找一段特征码（支持 ?? 和 * 通配）
local hits = AOBScan("48 8B 05 ?? ?? ?? ?? 48 85 C0")
if not hits or #hits == 0 then
  print("没有找到特征码，先附加一个进程再试。")
  return
end
print(string.format("找到 %d 处特征码，第一处 %X", #hits, hits[1]))

-- 2) 先读原值，再写。读出来的原值就是撤销的依据。
local target = hits[1]
local original = readBytes(target, 3)
print(string.format("原指令字节：%02X %02X %02X", original[1], original[2], original[3]))

writeBytes(target, { 0x90, 0x90, 0x90 }, 3)
local after = readBytes(target, 3)
print(string.format("写入后：%02X %02X %02X", after[1], after[2], after[3]))

-- 3) 按原值还原。样本自己收尾，重复运行也不会留下痕迹。
writeBytes(target, { original[1], original[2], original[3] }, 3)
local restored = readBytes(target, 3)
local same = restored[1] == original[1]
  and restored[2] == original[2]
  and restored[3] == original[3]
print(same and "已按原值还原。" or "还原失败，字节对不上。")
