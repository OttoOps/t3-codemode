import fnmatch
import sys
import tomllib

with open(sys.argv[1], "rb") as f:
    globs = tomllib.load(f).get("examine_globs", [])

for path in sys.stdin.read().splitlines():
    if any(fnmatch.fnmatchcase(path, glob) for glob in globs):
        print(path)
