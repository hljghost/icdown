"""Genera las imágenes para redes sociales del README a partir del panel real de la app.

Renderiza el panel de progreso con datos de ejemplo (`cargo run --example panel_social`, nada
personal) y lo compone sobre un fondo degradado:

  icopy-social.png         1920×1080 (16:9)
  icopy-social-square.png  1080×1080
  panel-oscuro.png / panel-claro.png  capturas sueltas del panel (2x)

Uso (desde cualquier carpeta; requiere Rust y Pillow, solo Windows por las fuentes y GDI):
  python docs/social/generar.py
"""
import random
import subprocess
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

AQUI = Path(__file__).resolve().parent
RAIZ = AQUI.parent.parent
CRATE = RAIZ / "icopia"
FUENTES = Path("C:/Windows/Fonts")

TITULO = "iCopy"
LEMA = ["Tus fotos de iCloud, en tu PC.", "Completas, verificadas y sin cortes."]
PUNTOS = [
    ("Todos los originales", "fotos, vídeos y Live Photos, ordenados por año y mes"),
    ("Verificación de cada archivo", "tamaño exacto, estructura y SHA-256"),
    ("Nunca pierde el hilo", "reanuda tras cortes, reinicios o enlaces caducados"),
]
PIE = "Para Windows  ·  Código abierto (MIT)  ·  github.com/Z1Code/iCopy"


def fuente(nombre, tam):
    return ImageFont.truetype(str(FUENTES / nombre), tam)


def panel(escala, tema):
    salida = AQUI / f"_panel_{tema}_{escala}.png"
    subprocess.run(
        ["cargo", "run", "-q", "--example", "panel_social", "--", str(salida), str(escala), tema],
        cwd=CRATE, check=True,
    )
    im = Image.open(salida).convert("RGBA")
    salida.unlink()
    return im


def mascara_redondeada(w, h, r, ss=4):
    m = Image.new("L", (w * ss, h * ss), 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, w * ss - 1, h * ss - 1), r * ss, fill=255)
    return m.resize((w, h), Image.LANCZOS)


def fondo(w, h, halo_caja):
    """Degradado índigo profundo con dos halos suaves y un grano finísimo (evita bandas)."""
    arriba, abajo = (14, 18, 46), (7, 9, 24)
    base = Image.new("RGB", (1, 256))
    for y in range(256):
        t = y / 255
        base.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(arriba, abajo)))
    im = base.resize((w, h), Image.BICUBIC).convert("RGBA")
    halo = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(halo)
    d.ellipse(halo_caja, fill=(40, 110, 255, 95))
    d.ellipse((-w * 0.20, -h * 0.45, w * 0.40, h * 0.40), fill=(120, 70, 255, 60))
    halo = halo.filter(ImageFilter.GaussianBlur(min(w, h) * 0.22))
    im = Image.alpha_composite(im, halo)
    rnd = random.Random(7)
    grano = Image.frombytes("L", (w, h), bytes(rnd.randrange(123, 133) for _ in range(w * h)))
    gris = Image.merge("RGB", (grano, grano, grano))
    im = ImageChops.add(im.convert("RGB"), gris, scale=1.0, offset=-128)
    return im.convert("RGBA")


def pegar_ventana(lienzo, ventana, x, y, radio):
    """Panel con esquinas redondeadas, borde sutil y sombra difusa."""
    w, h = ventana.size
    m = mascara_redondeada(w, h, radio)
    sombra = Image.new("RGBA", lienzo.size, (0, 0, 0, 0))
    for desp, borrosa, alfa in ((int(h * 0.05), int(h * 0.07), 150), (int(h * 0.01), int(h * 0.015), 110)):
        capa = Image.new("RGBA", (w, h), (0, 0, 0, alfa))
        s = Image.new("RGBA", lienzo.size, (0, 0, 0, 0))
        s.paste(capa, (x, y + desp), m)
        sombra = Image.alpha_composite(sombra, s.filter(ImageFilter.GaussianBlur(borrosa)))
    lienzo.alpha_composite(sombra)
    v = ventana.copy()
    v.putalpha(m)
    lienzo.alpha_composite(v, (x, y))
    borde = Image.new("RGBA", (w * 4, h * 4), (0, 0, 0, 0))
    ImageDraw.Draw(borde).rounded_rectangle((0, 0, w * 4 - 1, h * 4 - 1), radio * 4, outline=(255, 255, 255, 40), width=4)
    lienzo.alpha_composite(borde.resize((w, h), Image.LANCZOS), (x, y))


def icono(tam):
    ico = Image.open(CRATE / "assets" / "icopia.ico")
    ico.size = (256, 256)
    return ico.convert("RGBA").resize((tam, tam), Image.LANCZOS)


def check(d, cx, cy, r, ss=1):
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(64, 156, 255, 255))
    p = [(cx - r * 0.42, cy + r * 0.02), (cx - r * 0.10, cy + r * 0.34), (cx + r * 0.45, cy - r * 0.30)]
    d.line(p, fill=(255, 255, 255, 255), width=max(2, int(r * 0.24)), joint="curve")


def check_suave(lienzo, cx, cy, r):
    ss = 4
    capa = Image.new("RGBA", (r * 2 * ss + 8, r * 2 * ss + 8), (0, 0, 0, 0))
    c = r * ss + 4
    check(ImageDraw.Draw(capa), c, c, r * ss)
    capa = capa.resize((capa.width // ss, capa.height // ss), Image.LANCZOS)
    lienzo.alpha_composite(capa, (cx - capa.width // 2, cy - capa.height // 2))


def horizontal(ventana):
    W, H = 1920, 1080
    im = fondo(W, H, (W * 0.50, H * 0.05, W * 1.05, H * 0.95))
    d = ImageDraw.Draw(im)
    vx = W - ventana.width - 150
    vy = (H - ventana.height) // 2
    pegar_ventana(im, ventana, vx, vy, 18)

    x = 140
    lado = 104
    f_titulo = fuente("segoeuib.ttf", 112)
    f_lema = fuente("segoeuisl.ttf", 44)
    f_punto = fuente("seguisb.ttf", 31)
    f_sub = fuente("segoeui.ttf", 25)
    f_pie = fuente("segoeui.ttf", 22)

    y = 200
    im.alpha_composite(icono(lado), (x, y + 4))
    d.text((x + lado + 30, y + lado // 2 + 4), TITULO, font=f_titulo, fill=(255, 255, 255), anchor="lm")
    y += lado + 58
    for linea in LEMA:
        d.text((x, y), linea, font=f_lema, fill=(214, 224, 255), anchor="la")
        y += 60
    y += 54
    for titulo, sub in PUNTOS:
        check_suave(im, x + 18, y + 20, 17)
        d.text((x + 56, y + 20), titulo, font=f_punto, fill=(255, 255, 255), anchor="lm")
        d.text((x + 56, y + 58), sub, font=f_sub, fill=(160, 174, 214), anchor="lm")
        y += 108
    d.text((x, vy + ventana.height), PIE, font=f_pie, fill=(124, 136, 180), anchor="ld")
    return im.convert("RGB")


def cuadrada(ventana):
    W = H = 1080
    im = fondo(W, H, (W * 0.12, H * 0.30, W * 0.88, H * 1.05))
    d = ImageDraw.Draw(im)
    f_titulo = fuente("segoeuib.ttf", 76)
    f_lema = fuente("segoeuisl.ttf", 33)
    lado = 72
    ancho_t = d.textlength(TITULO, font=f_titulo)
    total = lado + 22 + ancho_t
    x0 = int((W - total) / 2)
    y = 56
    im.alpha_composite(icono(lado), (x0, y))
    d.text((x0 + lado + 22, y + lado // 2 + 2), TITULO, font=f_titulo, fill=(255, 255, 255), anchor="lm")
    y += lado + 26
    d.text((W / 2, y), " ".join(LEMA), font=f_lema, fill=(214, 224, 255), anchor="ma")
    y += 44
    vy = (y + H - ventana.height) // 2
    pegar_ventana(im, ventana, (W - ventana.width) // 2, vy, 14)
    return im.convert("RGB")


def main():
    oscuro_h = panel(2.25, "oscuro")
    horizontal(oscuro_h).save(AQUI / "icopy-social.png", optimize=True)
    oscuro_c = panel(1.95, "oscuro")
    cuadrada(oscuro_c).save(AQUI / "icopy-social-square.png", optimize=True)
    panel(2, "oscuro").save(AQUI / "panel-oscuro.png", optimize=True)
    panel(2, "claro").save(AQUI / "panel-claro.png", optimize=True)
    print("Listo:", ", ".join(p.name for p in sorted(AQUI.glob("*.png"))))


if __name__ == "__main__":
    main()
