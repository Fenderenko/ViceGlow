"""Effect tables of every lamp family → web/fx.json.

Usage: python tools/gen_fx.py <path to UniLED dev_v3 custom_components/uniled/lib>
BanlanX tables are read from UniLED (github.com/monty68/uniled, branch dev_v3); the others are
written out here from the projects listed in src/drv/mod.rs. Names are translated to Ukrainian
from English words; every effect gets a kind (for the filter chips) and colour letters (for the
preview): R G B Y C P W, "*" — many colours.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

COLOURS = [  # longest first
    ("seven color", "*"), ("seven colors", "*"), ("full color", "*"), ("rainbow", "*"), ("rgb", "RGB"), ("multicolor", "*"),
    ("red", "R"), ("green", "G"), ("blue", "B"), ("yellow", "Y"), ("cyan", "C"), ("purple", "P"), ("magenta", "P"), ("white", "W"),
]
UK_COL = {"R": "червоний", "G": "зелений", "B": "синій", "Y": "жовтий", "C": "блакитний", "P": "фіолетовий", "W": "білий"}

# (English keyword, kind, Ukrainian name); the first match wins
KINDS = [
    ("background stars", "stars", "Зорі на тлі"),
    ("twinkle", "stars", "Мерехтливі зорі"),
    ("stars", "stars", "Зорі"),
    ("star", "stars", "Зорі"),
    ("meteor", "meteor", "Метеор"),
    ("metor", "meteor", "Метеор"),
    ("comet spin", "spin", "Комета по колу"),
    ("dot spin", "spin", "Точка по колу"),
    ("segment spin", "spin", "Сегмент по колу"),
    ("spin", "spin", "По колу"),
    ("comet", "comet", "Комета"),
    ("gradual snake", "snake", "Плавна змійка"),
    ("snake", "snake", "Змійка"),
    ("chas", "snake", "Біг"),
    ("wiping", "snake", "Витирання"),
    ("scroll", "snake", "Біг"),
    ("stack", "stack", "Наповнення"),
    ("fill", "stack", "Наповнення"),
    ("heart", "pulse", "Серцебиття"),
    ("breath", "breath", "Дихання"),
    ("pulsating", "breath", "Дихання"),
    ("gradual change", "breath", "Дихання"),
    ("strobe", "strobe", "Спалахи"),
    ("flash", "strobe", "Спалахи"),
    ("blink", "strobe", "Спалахи"),
    ("jump", "jump", "Стрибки"),
    ("cross fade", "fade", "Перелив"),
    ("crossfade", "fade", "Перелив"),
    ("fade", "fade", "Перелив"),
    ("gradient", "fade", "Градієнт"),
    ("flow", "flow", "Потік"),
    ("wave", "wave", "Хвиля"),
    ("fire", "fire", "Вогонь"),
    ("candle", "fire", "Свічка"),
    ("segment", "spin", "Сегменти"),
    ("dots", "stars", "Кольорові точки"),
    ("block", "jump", "Кольорові блоки"),
    ("cycle", "rainbow", "Перебір кольорів"),
    ("ambulance", "strobe", "Мигалка"),
    ("solid", "solid", "Один колір"),
    ("switch", "jump", "Перемикання"),
    ("magic", "rainbow", "Магія"),
]

SOUND_KINDS = [
    ("spectrum", "Спектр"), ("stars", "Зорі"), ("energy", "Енергія"), ("pulse", "Пульс"), ("ejection", "Викид"), ("vumeter", "Індикатор"),
    ("vu meter", "Індикатор"), ("love", "Любов"), ("christmas", "Різдво"), ("heartbeat", "Серцебиття"), ("party", "Вечірка"), ("breath", "Дихання"),
    ("jump", "Стрибки"), ("beat injection", "Удари"), ("ripple", "Брижі"), ("music mode", "Музика"),
]


def colours(t):
    t = t.lower()
    out = ""
    for k, c in COLOURS:
        while k in t:
            out += c if c not in out or c == "*" else ""
            t = t.replace(k, " ", 1)
    return "*" if "*" in out else out


def uk(name, sound=False):
    """English effect name → (Ukrainian name, kind, colour letters)."""
    low = name.lower().replace("_", " ")
    cols = colours(low)
    if sound or low.startswith("sound") or low.startswith("music"):
        base = next((u for k, u in SOUND_KINDS if k in low), "Звук")
        if "full color" in low or "gradient" in low:
            base += " · усі кольори"
        elif "single" in low or "mono" in low:
            base += " · один колір"
        num = re.search(r"(\d+)\s*$", low)
        return (f"Звук · {base}" + (f" {num.group(1)}" if num and base == "Музика" else ""), "sound", cols or "*")
    kind, base = "other", None
    for k, kd, u in KINDS:
        if k in low:
            kind, base = kd, u
            break
    if "rainbow" in low and kind in ("other", "rainbow", "fade", "jump"):
        base = {"fade": "Веселка · перелив", "jump": "Веселка · стрибки"}.get(kind, "Веселка")
        kind = "rainbow"
    elif "rainbow" in low:
        base = f"Веселка · {base.lower()}"
        kind = "rainbow"
    if base is None and cols and not re.sub(r"red|green|blue|yellow|cyan|purple|magenta|white|[/ ,]", "", low):
        kind, base = "snake", "Біг"
    if base is None:
        num = re.search(r"(\d+)\s*$", low)
        return (f"Ефект {num.group(1)}" if num else name, "other", cols)
    if cols == "*" and kind not in ("rainbow",):
        base += " · 7 кольорів"
    elif cols and kind != "rainbow":
        base += " · " + " / ".join(UK_COL[c] for c in cols)
    if kind in ("fade", "jump") and cols == "*":
        kind = "rainbow"
    return base, kind, cols


def entry(i, name, sound=False):
    n, k, c = uk(name, sound)
    return {"id": i, "name": n, "kind": k, "cols": c}


def enum_names(lib):
    txt = (lib / "effects.py").read_text(encoding="utf-8")
    return dict(re.findall(r"^\s+([A-Z0-9_]+)\s*=\s*\"([^\"]+)\"", txt, re.M))


def py_dict(txt, name, enum, base=0, sound=False):
    """Entries of `NAME: Final = { key: _FX_…(UNILEDEffects.X | "text" …) }`."""
    m = re.search(name + r": Final = \{(.*?)\n    \}", txt, re.S)
    if not m:
        m = re.search(name + r": Final = \{(.*?)\n\}", txt, re.S)
    body = m.group(1)
    out = []
    for key, ref, lit in re.findall(r"(0x[0-9A-Fa-f]+|[A-Z0-9_]+ \+ \d+)\s*:\s*_FX_\w+\(\s*(?:UNILEDEffects\.(\w+)|\"([^\"]+)\")?", body):
        if " + " in key:
            const, off = key.split(" + ")
            k = CONST[const] + int(off)
        else:
            k = int(key, 16)
        nm = enum.get(ref, lit) if ref or lit else "Solid"
        out.append(entry(base | k, nm, sound or nm.startswith("Sound")))
    return out


CONST = {"BANLANX601_EFFECT_PATTERN": 0x01, "BANLANX601_EFFECT_SOUND": 0x65, "BANLANX601_EFFECT_SOLID": 0x19}


def banlanx2():
    """The SP621E table as in Glowo 0.1.0 (checked against the lamp's effect ids)."""
    one = list("RGBYCPW")
    pairs = ["RG", "RB", "RY", "RC", "RP", "RW", "GB", "GY", "GC", "GP", "GW", "BY", "BC", "BP", "BW", "YC", "YP", "YW", "CP", "CW", "PW"]
    seq = lambda base, keys: [(base + i, k) for i, k in enumerate(keys)]
    groups = [
        ("Веселка", "rainbow", [(1, "*", "Веселка"), (2, "*", "Веселка · метеор"), (3, "*", "Веселка · зорі"), (4, "*", "Веселка · по колу")]),
        ("Вогонь", "fire", seq(0x05, ["RY", "RP", "GY", "GC", "BP", "BC"])),
        ("Комета", "comet", seq(0x0B, one)),
        ("Метеор", "meteor", seq(0x12, one)),
        ("Перелив", "fade", seq(0x19, pairs)),
        ("Хвиля", "wave", seq(0x2E, one) + seq(0x35, pairs)),
        ("Зорі", "stars", seq(0x4A, one)),
        ("Зорі на тлі", "stars", seq(0x51, list("RGBYCP")) + seq(0x57, ["RW", "GW", "BW", "YW", "CW", "PW", "W"])),
        ("Дихання", "breath", seq(0x5E, one)),
        ("Наповнення", "stack", seq(0x65, one) + [(0x6C, "*", "Наповнення · усі кольори")] + seq(0x6D, ["RG", "GB", "BY", "YC", "CP", "PW"])),
        ("Змійка", "snake", seq(0x73, ["RBW", "GYW", "RGW", "RY", "RW", "GW"])),
        ("Комета по колу", "spin", seq(0x79, one)),
        ("Точка по колу", "spin", seq(0x80, one)),
        ("Сегмент по колу", "spin", seq(0x87, one)),
        ("Градієнт", "rainbow", [(0x8E, "*", "Градієнт")]),
    ]
    out = []
    for gname, kind, items in groups:
        for it in items:
            i, c = it[0], it[1]
            name = it[2] if len(it) > 2 else gname + " · " + " / ".join(UK_COL[x] for x in c)
            out.append({"id": i, "name": name, "kind": kind, "cols": c})
    return out


EN_COL = {"червоний": "red", "зелений": "green", "синій": "blue", "жовтий": "yellow", "блакитний": "cyan", "фіолетовий": "purple", "білий": "white"}
EN_PART = {
    "Веселка": "Rainbow", "метеор": "meteor", "зорі": "stars", "по колу": "spin", "перелив": "blend", "стрибки": "jump",
    "Вогонь": "Fire", "Комета": "Comet", "Метеор": "Meteor", "Перелив": "Blend", "Хвиля": "Wave", "Зорі": "Stars", "Зорі на тлі": "Stars on background",
    "Мерехтливі зорі": "Twinkling stars", "Дихання": "Breathing", "Наповнення": "Fill", "Змійка": "Snake", "Плавна змійка": "Smooth snake", "Біг": "Chase",
    "Витирання": "Wipe", "Комета по колу": "Comet spin", "Точка по колу": "Dot spin", "Сегмент по колу": "Segment spin", "По колу": "Spin", "Сегменти": "Segments",
    "Кольорові точки": "Colour dots", "Кольорові блоки": "Colour blocks", "Перебір кольорів": "Colour cycle", "Мигалка": "Siren", "Один колір": "Solid",
    "Перемикання": "Switching", "Магія": "Magic", "Серцебиття": "Heartbeat", "Спалахи": "Strobe", "Стрибки": "Jump", "Градієнт": "Gradient", "Потік": "Flow",
    "Свічка": "Candle", "Звук": "Sound", "Спектр": "Spectrum", "Енергія": "Energy", "Пульс": "Pulse", "Викид": "Burst", "Індикатор": "Level meter",
    "Любов": "Love", "Різдво": "Christmas", "Вечірка": "Party", "Удари": "Beats", "Брижі": "Ripple", "Музика": "Music", "усі кольори": "all colours",
    "один колір": "one colour", "7 кольорів": "7 colours", "Усі ефекти по черзі": "All effects in turn",
    "Океан": "Ocean", "Романтика": "Romance", "Захід сонця": "Sunset", "Камін": "Fireplace", "Затишок": "Cozy", "Ліс": "Forest", "Пастель": "Pastel",
    "Пробудження": "Wake up", "Перед сном": "Bedtime", "Теплий білий": "Warm white", "Денне світло": "Daylight", "Холодний білий": "Cool white",
    "Нічник": "Night light", "Зосередження": "Focus", "Відпочинок": "Relax", "Справжні кольори": "True colours", "Телевізор": "TV time", "Рослини": "Plant growth",
    "Весна": "Spring", "Літо": "Summer", "Осінь": "Fall", "Глибина": "Deep dive", "Джунглі": "Jungle", "Мохіто": "Mojito", "Клуб": "Club", "Хелловін": "Halloween",
    "Свічки": "Candlelight", "Золотий білий": "Golden white", "Стимпанк": "Steampunk",
}
EN_NUM = {"Ефект": "Effect", "смуга": "strip", "матриця": "matrix", "Музика": "Music"}


def english(name):
    """The English name of a generated Ukrainian one (part by part)."""
    out = []
    for part in name.split(" · "):
        if part in EN_PART:
            out.append(EN_PART[part])
        elif part[:1].upper() + part[1:] in EN_PART:
            out.append(EN_PART[part[:1].upper() + part[1:]].lower())
        elif all(c in EN_COL for c in part.split(" / ")):
            out.append(" / ".join(EN_COL[c] for c in part.split(" / ")))
        elif (m := re.fullmatch(r"(\D+?) (\d+)", part)) and m.group(1) in EN_NUM:
            out.append(f"{EN_NUM[m.group(1)]} {m.group(2)}")
        else:
            raise SystemExit(f"no English for {ascii(part)} in {ascii(name)}")
    return " · ".join(out)


def named(pairs, base_kind=None):
    return [entry(i, n) if base_kind is None else {"id": i, "name": n, "kind": base_kind, "cols": ""} for i, n in pairs]


def main():
    lib = Path(sys.argv[1])
    enum = enum_names(lib)
    st = (lib / "sptech_effects.py").read_text(encoding="utf-8")
    s601 = (lib / "ble" / "sp601e.py").read_text(encoding="utf-8")
    b3 = [(0x63, "Solid Color")] + [(1 + i, n) for i, n in enumerate(["Seven Color Gradient", "Seven Color Jump", "Seven Color Breath", "Seven Color Strobe", "Red Breath", "Green Breath", "Blue Breath", "Yellow Breath", "Cyan Breath", "Purple Breath", "White Breath", "Red Strobe", "Green Strobe", "Blue Strobe", "Yellow Strobe", "Cyan Strobe", "Purple Strobe", "White Strobe"])] + [(32, "Breath"), (33, "Strobe")]
    b3 = [x for x in b3 if x[0] != 0x63]
    b3_sound = [(0x65, "Sound - Music Breath"), (0x66, "Sound - Music Jump"), (0x67, "Sound - Monochrome Music Breath")]
    t = {
        "banlanx2": banlanx2(),
        "banlanx3": named(b3),
        "banlanx3m": named(b3) + [entry(i, n, True) for i, n in b3_sound],
        "sp110e": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": ""} for k in range(1, 0x79)],
        "sp107e": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": ""} for k in range(1, 181)] + [{"id": 0xBE + k, "name": f"Звук · смуга {k + 1}", "kind": "sound", "cols": "*"} for k in range(18)] + [{"id": 0xDC + k, "name": f"Звук · матриця {k + 1}", "kind": "sound", "cols": "*"} for k in range(30)],
        "sp601e": [e for e in py_dict(s601, "BANLANX601_EFFECTS", enum) if e["id"] != 0x19],
        "sptech_spi": py_dict(st, "DICTOF_SPI_EFFECTS_DYNAMIC_COLOR", enum, 0x300) + py_dict(st, "DICTOF_SPI_EFFECTS_SOUND_COLOR", enum, 0x500, True),
        "sptech_pwm": py_dict(st, "DICTOF_PWM_EFFECTS_DYNAMIC_COLOR", enum, 0x300) + py_dict(st, "DICTOF_PWM_EFFECTS_SOUND_COLOR", enum, 0x500, True),
        "triones": named([(0x25, "Seven Color Cross Fade"), (0x26, "Red Gradual Change"), (0x27, "Green Gradual Change"), (0x28, "Blue Gradual Change"), (0x29, "Yellow Gradual Change"), (0x2A, "Cyan Gradual Change"), (0x2B, "Purple Gradual Change"), (0x2C, "White Gradual Change"), (0x2D, "Red Green Cross Fade"), (0x2E, "Red Blue Cross Fade"), (0x2F, "Green Blue Cross Fade"), (0x30, "Seven Color Strobe Flash"), (0x31, "Red Strobe Flash"), (0x32, "Green Strobe Flash"), (0x33, "Blue Strobe Flash"), (0x34, "Yellow Strobe Flash"), (0x35, "Cyan Strobe Flash"), (0x36, "Purple Strobe Flash"), (0x37, "White Strobe Flash"), (0x38, "Seven Color Jumping")]),
        "dream": [{"id": k, "name": f"Ефект {k + 1}", "kind": "other", "cols": ""} for k in range(255)],
        "elk": named([(135, "RGB jump"), (136, "Seven color jump"), (137, "RGB crossfade"), (138, "Seven color crossfade"), (139, "Red crossfade"), (140, "Green crossfade"), (141, "Blue crossfade"), (142, "Yellow crossfade"), (143, "Cyan crossfade"), (144, "Magenta crossfade"), (145, "White crossfade"), (146, "Red green crossfade"), (147, "Red blue crossfade"), (148, "Green blue crossfade"), (149, "Seven color blink"), (150, "Red blink"), (151, "Green blink"), (152, "Blue blink"), (153, "Yellow blink"), (154, "Cyan blink"), (155, "Magenta blink"), (156, "White blink")]),
        "melk": named([(0, "Switches all"), (1, "Soft fade all"), (5, "RGB fade"), (10, "Chase all"), (16, "Fast chase all"), (22, "Fade all"), (26, "Red fade"), (28, "Green fade"), (33, "Cyan fade"), (58, "Chase all colors"), (75, "White chase"), (77, "Chase all slow"), (151, "White blue white chase left"), (152, "White blue white chase right"), (155, "Red white red chase left"), (156, "Red white red chase right")]),
        "melk_ox": named([(0, "Cycle"), (1, "Magic back"), (2, "Effect 2"), (3, "Effect 3"), (4, "Effect 4"), (5, "Effect 5"), (16, "Rainbow cycle"), (32, "Color wave"), (48, "Breathing"), (64, "Strobe"), (128, "RGB jump"), (144, "RGB fade"), (207, "Blue scroll")]),
        "dmrrba": named([(1, "Flash"), (2, "RGB fade"), (3, "Rainbow fade"), (4, "RGB jump"), (5, "Rainbow jump"), (6, "Breath"), (8, "Candle"), (9, "Ambulance")]),
        "keepsmile": named([(0x80, "Seven color jumping change"), (0x81, "RGB jumping change"), (0x82, "Pulsating Seven colors"), (0x83, "Pulsating RGB"), (0x84, "Pulsating red"), (0x85, "Pulsating green"), (0x86, "Pulsating blue"), (0x87, "Pulsating yellow"), (0x88, "Pulsating cyan"), (0x89, "Pulsating purple"), (0x8A, "Pulsating white"), (0x8B, "Pulsating red,green"), (0x8C, "Pulsating red,blue"), (0x8D, "Pulsating green,blue"), (0x8E, "Seven color strobe flash"), (0x8F, "RGB strobe"), (0x90, "Red strobe flash"), (0x91, "Green strobe flash"), (0x92, "Blue strobe flash"), (0x93, "Yellow strobe flash"), (0x94, "Cyan strobe flash"), (0x95, "Purple strobe flash"), (0x96, "White strobe flash")]),
        "mohuan": named([(0x00, "Colorloop cycle"), (0x01, "Red fade"), (0x02, "Green fade"), (0x03, "Blue fade"), (0x04, "Yellow fade"), (0x05, "Cyan fade"), (0x06, "Magenta fade"), (0x07, "White fade"), (0x08, "Red green cross fade"), (0x09, "Red blue cross fade"), (0x0A, "Green blue cross fade"), (0x0B, "Rainbow fade"), (0x0C, "Seven color strobe"), (0x0D, "Red strobe"), (0x0E, "Green strobe"), (0x0F, "Blue strobe"), (0x10, "Yellow strobe"), (0x11, "Cyan strobe"), (0x12, "Magenta strobe"), (0x13, "White strobe"), (0x14, "Seven color jump"), (0x15, "RGB jump")]),
        "ideal": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": "*"} for k in range(1, 11)],
        "ledlamp": [{"id": k, "name": f"Ефект {k - 0x86}", "kind": "other", "cols": ""} for k in range(0x87, 0x9E)],
        "zengge_53": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": ""} for k in range(1, 114)] + [{"id": 255, "name": "Усі ефекти по черзі", "kind": "rainbow", "cols": "*"}],
        "zengge_sym": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": ""} for k in range(1, 101)],
        "zengge_iotbt": [{"id": k, "name": f"Ефект {k}", "kind": "other", "cols": ""} for k in range(1, 13)],
        "wiz": [{"id": i + 1, "name": n, "kind": k, "cols": c} for i, (n, k, c) in enumerate([
            ("Океан", "wave", "BC"), ("Романтика", "breath", "PR"), ("Захід сонця", "fade", "RY"), ("Вечірка", "rainbow", "*"), ("Камін", "fire", "RY"), ("Затишок", "breath", "Y"),
            ("Ліс", "fade", "GY"), ("Пастель", "fade", "*"), ("Пробудження", "fade", "YW"), ("Перед сном", "fade", "Y"), ("Теплий білий", "solid", "Y"), ("Денне світло", "solid", "W"),
            ("Холодний білий", "solid", "W"), ("Нічник", "solid", "Y"), ("Зосередження", "solid", "W"), ("Відпочинок", "solid", "Y"), ("Справжні кольори", "solid", "W"), ("Телевізор", "solid", "B"),
            ("Рослини", "solid", "RP"), ("Весна", "fade", "GY"), ("Літо", "fade", "YG"), ("Осінь", "fade", "RY"), ("Глибина", "wave", "B"), ("Джунглі", "fade", "G"),
            ("Мохіто", "fade", "GY"), ("Клуб", "strobe", "*"), ("Різдво", "fade", "RG"), ("Хелловін", "fade", "RP"), ("Свічки", "fire", "Y"), ("Золотий білий", "solid", "Y"),
            ("Пульс", "breath", "*"), ("Стимпанк", "fire", "RY")])],
    }
    t["zengge_simple"] = t["triones"]  # the same presets, ids 37..56
    for table in t.values():
        for e in table:
            e["en"] = english(e["name"])
    out = ROOT / "web" / "fx.json"
    out.write_text(json.dumps(t, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8", newline="\n")
    print(f"{out}: {sum(len(v) for v in t.values())} effects in {len(t)} tables")


if __name__ == "__main__":
    main()
