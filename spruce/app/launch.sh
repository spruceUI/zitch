#!/bin/sh
# zitch, itch.io's handheld frontend, from spruceUI/zitch. It draws through
# the device's own libSDL2, so each platform points it at the SDL build PyUI
# uses there, as Cheevos does.

. /mnt/SDCARD/spruce/scripts/helperFunctions.sh

APP_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG="/mnt/SDCARD/Saves/spruce/zitch-$PLATFORM.log"

case "$PLATFORM" in
    "Flip")
        export LD_LIBRARY_PATH="/mnt/SDCARD/App/PyUI/dll:$LD_LIBRARY_PATH"
        ;;
    "Brick" | "BrickPro" | "SmartPro" | "SmartProS")
        export LD_LIBRARY_PATH="/mnt/SDCARD/spruce/brick/sdl2:$LD_LIBRARY_PATH"
        ;;
    "AnbernicXX"* | "AnbernicRG28XX" | "AnbernicRGCubeXX")
        PYUI_DLL=/mnt/SDCARD/App/PyUI/dll
        [ -f /mnt/SDCARD/App/PyUI/dll-mali/libSDL2-2.0.so.0 ] && PYUI_DLL=/mnt/SDCARD/App/PyUI/dll-mali
        export LD_LIBRARY_PATH="$PYUI_DLL:/mnt/SDCARD/spruce/flip/lib:/usr/lib"
        export SDL_JOYSTICK_DISABLE_UDEV=1
        case "$PYUI_DLL" in
            *dll-mali) export SDL_VIDEODRIVER=mali ;;
        esac
        ;;
    "Pixel2")
        export LD_LIBRARY_PATH="/usr/lib:$LD_LIBRARY_PATH"
        ;;
    "Zero28" | "Zero40" | "XU20")
        export LD_LIBRARY_PATH="$(magicx_pyui_sdl_dir):$LD_LIBRARY_PATH"
        ;;
    "Miniloong" | "RGB30" | "RGB20SX" | "A10Mini" | "A10MiniV4" | "XUMiniM" | "G350" | "V10")
        # dArkMoss's own SDL2 2.32 (KMSDRM, Mali GLES).
        export LD_LIBRARY_PATH="/usr/lib/aarch64-linux-gnu:$LD_LIBRARY_PATH"
        export SDL_VIDEODRIVER=kmsdrm
        export SDL_AUDIODRIVER=alsa
        ;;
    *)
        /mnt/SDCARD/App/PyUI/launch.sh -msgDisplay "itch.io doesn't support this device ($PLATFORM) yet." -msgDisplayTimeMs 5000
        exit 1
        ;;
esac

# The login, butler's database and downloaded games live in Saves, which
# updates never touch.
HOME="/mnt/SDCARD/Saves/zitch"
XDG_CONFIG_HOME="$HOME/.config"
XDG_CACHE_HOME="$HOME/.cache"
export HOME XDG_CONFIG_HOME XDG_CACHE_HOME
mkdir -p "$HOME"

export_sdl_gamecontroller_map positional
export SSL_CERT_FILE=/mnt/SDCARD/spruce/etc/ca-certificates.crt

# Downloads run long; idle sleep would cut them off.
echo 1 > /tmp/stay_awake

cd "$APP_DIR/zitch" || exit 1
chmod +x zitch butler love
./zitch --fullscreen --low-spec --butler "$APP_DIR/zitch/butler" >"$LOG" 2>&1

rm -f /tmp/stay_awake
