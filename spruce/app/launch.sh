#!/bin/sh
# zitch, itch.io's handheld frontend, from spruceUI/zitch. It draws through
# the device's own libSDL2, so each platform points it at the SDL build PyUI
# uses there, as Cheevos does.

. /mnt/SDCARD/spruce/scripts/helperFunctions.sh

APP_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG="/mnt/SDCARD/Saves/spruce/zitch-$PLATFORM.log"
# A game can't share the screen with zitch, so zitch writes what to run here
# and exits; it runs below, then zitch comes back.
HANDOFF=/tmp/zitch_next.sh

# zitch's environment only: the games it hands over run with spruce's.
zitch_env() {
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
            LD_LIBRARY_PATH="$(magicx_pyui_sdl_dir):$LD_LIBRARY_PATH"
            export LD_LIBRARY_PATH
            ;;
        "Miniloong" | "RGB30" | "RGB20SX" | "A10Mini" | "A10MiniV4" | "XUMiniM" | "G350" | "V10")
            # dArkMoss's own SDL2 2.32 (KMSDRM, Mali GLES).
            export LD_LIBRARY_PATH="/usr/lib/aarch64-linux-gnu:$LD_LIBRARY_PATH"
            export SDL_VIDEODRIVER=kmsdrm
            export SDL_AUDIODRIVER=alsa
            ;;
        *)
            return 1
            ;;
    esac

    # The login, butler's database and downloaded games live in Saves,
    # which updates never touch.
    HOME="/mnt/SDCARD/Saves/zitch"
    XDG_CONFIG_HOME="$HOME/.config"
    XDG_CACHE_HOME="$HOME/.cache"
    export HOME XDG_CONFIG_HOME XDG_CACHE_HOME
    mkdir -p "$HOME"

    export_sdl_gamecontroller_map positional
    # spruce devices are labelled the Nintendo way: A right, B bottom.
    export ZITCH_NINTENDO_BUTTONS=1
    export SSL_CERT_FILE=/mnt/SDCARD/spruce/etc/ca-certificates.crt
}

if ! (zitch_env); then
    /mnt/SDCARD/App/PyUI/launch.sh -msgDisplay "itch.io doesn't support this device ($PLATFORM) yet." -msgDisplayTimeMs 5000
    exit 1
fi

chmod +x "$APP_DIR/zitch/zitch" "$APP_DIR/zitch/butler" "$APP_DIR/zitch/love"

# The launchers zitch writes to Roms/ITCH run a LÖVE game through here, so
# the SDL setup always matches the device the card is in.
if [ "$1" = "--love" ]; then
    shift
    (
        zitch_env
        cd "$APP_DIR/zitch" || exit 1
        export LD_LIBRARY_PATH="$APP_DIR/zitch/libs:$APP_DIR/zitch/gl4es:$LD_LIBRARY_PATH"
        export LOVE_GRAPHICS_USE_OPENGLES=1
        # LÖVE's error screen only quits on Escape; Select+Start always ends it.
        /mnt/SDCARD/spruce/bin64/gptokeyb -k love -c "$APP_DIR/love.gptk" &
        gptk=$!
        ./love "$@"
        kill -9 "$gptk" 2>/dev/null
    ) >>"/mnt/SDCARD/Saves/spruce/zitch-love.log" 2>&1
    exit $?
fi

: >"$LOG"

while :; do
    rm -f "$HANDOFF"
    # Downloads run long; idle sleep would cut them off.
    echo 1 > /tmp/stay_awake
    (
        zitch_env
        cd "$APP_DIR/zitch" || exit 1
        ./zitch --fullscreen --low-spec --butler "$APP_DIR/zitch/butler"
    ) >>"$LOG" 2>&1
    rm -f /tmp/stay_awake
    [ -s "$HANDOFF" ] || break
    log_message "zitch: running $(cat "$HANDOFF")"
    sh "$HANDOFF"
done

rm -f "$HANDOFF"
