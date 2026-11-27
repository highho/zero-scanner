"""End-to-end target for Zero: a real process with known, addressable memory."""
import ctypes
import os
import sys
import time

INFO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "e2e_target.info")

# A real 4-byte little-endian value the scanner must find.
# The constants must stay in sync with the checks in e2e.rs.
COINS = 9900  # 0x26AC little-endian -> AC 26 00 00
HEALTH = 100
coins = ctypes.c_int32(COINS)
health = ctypes.c_int32(HEALTH)

# A pointer chain the pointer scanner must resolve.
coins_ptr = ctypes.c_void_p(ctypes.addressof(coins))
coins_pp = ctypes.c_void_p(ctypes.addressof(coins_ptr))

# A recognisable byte signature for the AOB scanner.
MarkerArray = ctypes.c_uint8 * 6
marker = MarkerArray(0x5A, 0x5A, 0x5A, 0x5A, 0xC3, 0x90)

def main() -> None:
    with open(INFO, "w", encoding="utf-8") as handle:
        handle.write("pid=%d\n" % os.getpid())
        handle.write("coins=%X\n" % ctypes.addressof(coins))
        handle.write("health=%X\n" % ctypes.addressof(health))
        handle.write("coins_ptr=%X\n" % ctypes.addressof(coins_ptr))
        handle.write("coins_pp=%X\n" % ctypes.addressof(coins_pp))
        handle.write("marker=%X\n" % ctypes.addressof(marker))
        handle.write("python=%d\n" % sys.version_info[0])
    # Count iterations so the speed-hack test can observe a slowdown.
    count = 0
    started = time.time()
    while True:
        count += 1
        if count % 20000 == 0:
            with open(os.path.join(os.path.dirname(INFO), "e2e_tick.txt"), "w") as handle:
                handle.write("%d\n" % count)
        time.sleep(0.0002)

if __name__ == "__main__":
    main()