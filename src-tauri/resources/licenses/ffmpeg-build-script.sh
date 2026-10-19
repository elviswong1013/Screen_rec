#!/bin/bash
set -xe

FFMPEG_COMMIT="46d899d7a02bc63237472cc4897258d5ddf274ac"
BUILD_DATE="$(date +%Y%m%d)"

cd /ffbuild
rm -rf ffmpeg prefix out
mkdir -p out/bin out/doc out/presets

git clone --filter=blob:none --single-branch --branch=release/8.1 https://github.com/FFmpeg/FFmpeg.git ffmpeg
cd ffmpeg
git checkout "$FFMPEG_COMMIT"
git describe --tags --always

# Remove only chromaprint; keep the rest of the BtbN LGPL configure set unchanged.
CONFIGURE_FLAGS="${FF_CONFIGURE/--enable-chromaprint/}"

./configure --prefix=/ffbuild/prefix --pkg-config-flags="--static" $FFBUILD_TARGET_FLAGS $CONFIGURE_FLAGS \
    --extra-cflags="$FF_CFLAGS" --extra-cxxflags="$FF_CXXFLAGS" --extra-libs="$FF_LIBS" \
    --extra-ldflags="$FF_LDFLAGS" --extra-ldexeflags="$FF_LDEXEFLAGS" \
    --cc="$CC" --cxx="$CXX" --ar="$AR" --ranlib="$RANLIB" --nm="$NM" \
    --extra-version="$BUILD_DATE"

make -j"$(nproc)" V=1
make install install-doc

cp /ffbuild/prefix/bin/ffmpeg.exe /ffbuild/out/bin/
cp /ffbuild/prefix/bin/ffprobe.exe /ffbuild/out/bin/
cp -r /ffbuild/prefix/share/doc/ffmpeg/* /ffbuild/out/doc/
cp /ffbuild/prefix/share/ffmpeg/*.ffpreset /ffbuild/out/presets/
cp COPYING.LGPLv3 /ffbuild/out/LICENSE.txt

{
    echo "Screen_rec custom FFmpeg build"
    echo "FFmpeg commit: $FFMPEG_COMMIT"
    echo "Build date: $BUILD_DATE"
    echo "Base image: ghcr.io/btbn/ffmpeg-builds/win64-lgpl-8.1:latest"
    echo "Configure flags: $CONFIGURE_FLAGS"
    echo "Removed: --enable-chromaprint (and its FFTW dependency)"
} > /ffbuild/out/BUILD-INFO.txt
