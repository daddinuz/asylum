import statistics

from collections import Counter


def longest_consecutive_item[T](l: list[T]) -> (T, int):
    current_item, current_count = None, 0
    longest_item, longest_count = None, 0
    while l:
        current_item = l[0]
        while l:
            if current_item == l[0]:
                current_count += 1
                l = l[1:]
            else:
                if current_count > longest_count:
                    longest_item, longest_count = current_item, current_count
                current_count = 0
                break
    return (longest_item, longest_count)


with open("english.txt") as f:
    words = f.read().splitlines()


def f(w: str) -> int:
    x = (ord(w[0]) << 24) | (ord(w[len(w) // 2]) << 16) | (ord(w[-1]) << 8) | len(w)
    x ^= x >> 19
    x ^= x >> 13
    x ^= x >> 5
    return x % 64


c = Counter(map(f, words))
l = list(f(w) for w in words)
m = min(c.values())
M = max(c.values())

print(f"len: {len(c)} max: {M} min: {m} delta: {M - m} mean: {statistics.fmean(c.values())} longest: {longest_consecutive_item(l)}")
print(c)
