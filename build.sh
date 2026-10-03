#!/bin/sh
set -e
cd "$(dirname "$0")"

case "$(uname -s)" in
    Darwin) EXT=dylib ;;
    MINGW*|MSYS*|CYGWIN*) EXT=dll ;;
    *) EXT=so ;;
esac

# A renamed crate leaves its old artefact in target/; drop it so it is not collected as current.
rm -f bin/target/release/*."$EXT"

# -p: a whole-workspace build unifies client-core's default feature-rdesk (OpenH264) back on.
cargo build --release -p ic-node-in-net-fs
mkdir -p bin
rm -f "bin/"*."$EXT"

found=0
for library in bin/target/release/*."$EXT"; do
    [ -f "$library" ] || continue
    cp "$library" bin/
    found=$((found + 1))
    printf '%s\n' "  $(basename "$library")"
done

if [ "$found" -eq 0 ]; then
    echo "no plugin libraries were produced" >&2
    exit 1
fi
echo "$found plugin(s) in bin/"
