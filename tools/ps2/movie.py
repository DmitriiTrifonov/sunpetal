#!/usr/bin/env python3
"""Читатель видеоархива PS2 `MOVIE.BIN` (только чтение).

Формат (по разведке, см. docs/PS2-RECON.md, раздел «Видео»):

    u16 'LF', u16 count, u32 version(1)
    count x { u32 size, u32 id }    size = длина потока без выравнивания
    потоки с 0x800, каждый выровнен по 0x800

Поток: MPEG-2 Program Stream (PSS). Пачки (pack, 00 00 01 BA) по 0x4000 байт;
видео MPEG-2 (PES 0xE0), звук в private stream 1 (0xBD) с под-потоком 0xA0:
после PES-заголовка 4 байта `FF A0 xx xx`, дальше блоки `SShd` (параметры) и `SSbd` (данные).
SShd: u32 'SShd', u32 len(0x18), u32 codec(1 = PCM s16le), u32 rate(48000), u32 channels(2), u32 interleave(0x200:
блоки по 512 байт L, потом R).

    movie.py MOVIE.BIN                    список потоков
    movie.py MOVIE.BIN video N OUT.m2v    видеопоток N (элементарный MPEG-2, ffmpeg читает)
    movie.py MOVIE.BIN audio N [T] OUT.wav  звук N, дорожка T (0 или 1), PCM 16 бит
    movie.py MOVIE.BIN pss N OUT.pss      поток N целиком (ffmpeg -i OUT.pss)
"""
import struct
import sys

ALIGN = 0x800


def entries(f):
    f.seek(0)
    h = f.read(8)
    if h[:2] != b'LF':
        raise SystemExit('не LF-архив')
    count = struct.unpack_from('<H', h, 2)[0]
    tab = f.read(8 * count)
    pos = ALIGN
    out = []
    for i in range(count):
        size, ident = struct.unpack_from('<II', tab, 8 * i)
        out.append((i, pos, size, ident))
        pos += (size + ALIGN - 1) // ALIGN * ALIGN
    return out


def packets(data):
    """Генератор (stream_id, payload) по PES-пакетам MPEG-PS."""
    p, n = 0, len(data)
    while p + 4 <= n:
        if data[p:p + 3] != b'\0\0\1':
            nxt = data.find(b'\0\0\1', p + 1)
            if nxt < 0:
                return
            p = nxt
            continue
        sid = data[p + 3]
        if sid == 0xBA:
            p += 14 + (data[p + 13] & 7)
        elif sid == 0xB9:
            return
        elif sid >= 0xBB:
            ln = struct.unpack_from('>H', data, p + 4)[0]
            body = data[p + 6:p + 6 + ln]
            if sid in (0xE0, 0xBD) and len(body) > 3:
                yield sid, body[3 + body[2]:]
            p += 6 + ln
        else:
            p += 4


def audio_tracks(data):
    """{номер дорожки: (параметры SShd, байты PCM)} из private stream 1.

    Заголовок под-потока `FF A0 00 NN`: NN = номер дорожки (в обоих роликах их две).
    SSbd.len = полная длина данных дорожки, дальше данные идут по пакетам без заголовков.
    """
    params, pcm = {}, {}
    for sid, pl in packets(data):
        if sid != 0xBD or pl[:2] != b'\xFF\xA0':
            continue
        tr, b = pl[3], pl[4:]
        pcm.setdefault(tr, bytearray())
        while b[:4] in (b'SShd', b'SSbd'):
            ln = struct.unpack_from('<I', b, 4)[0]
            if b[:4] == b'SShd':
                c, r, ch, i = struct.unpack_from('<4I', b, 8)
                params[tr] = dict(codec=c, rate=r, ch=ch, inter=i)
                b = b[8 + ln:]
            else:
                pcm[tr] += b[8:]
                b = b''
        pcm[tr] += b
    return {t: (params.get(t), bytes(v)) for t, v in pcm.items()}


def deinterleave(pcm, ch, inter):
    """Блоки по `inter` байт на канал (L, R, L, R...) -> обычное чередование отсчётов."""
    if ch == 1:
        return pcm
    frame = ch * inter
    out = bytearray(len(pcm) // frame * frame)
    for blk in range(len(pcm) // frame):
        base = blk * frame
        chans = [pcm[base + c * inter: base + (c + 1) * inter] for c in range(ch)]
        for c in range(ch):
            for k in range(0, inter, 2):
                o = base + (k // 2 * ch + c) * 2
                out[o:o + 2] = chans[c][k:k + 2]
    return bytes(out)


def load(f, e):
    f.seek(e[1])
    return f.read(e[2])


def main(argv):
    f = open(argv[0], 'rb')
    es = entries(f)
    if len(argv) == 1:
        for i, off, size, ident in es:
            d = load(f, es[i])
            vid = sum(len(x) for s, x in packets(d) if s == 0xE0)
            tr = audio_tracks(d)
            au = {t: (len(v[1]), v[0]) for t, v in tr.items()}
            print(f'#{i} off={off:#x} size={size} id={ident:#x} video={vid} audio={au}')
        return
    cmd, n, out = argv[1], int(argv[2]), argv[-1]
    d = load(f, es[n])
    if cmd == 'pss':
        open(out, 'wb').write(d)
    elif cmd == 'video':
        open(out, 'wb').write(b''.join(x for s, x in packets(d) if s == 0xE0))
    elif cmd == 'audio':
        track = int(argv[3]) if len(argv) > 4 else 0
        par, pcm = audio_tracks(d)[track]
        pcm = deinterleave(pcm, par['ch'], par['inter'])
        if par['codec'] != 1:
            print('codec', par['codec'], 'не PCM, пишу как есть', file=sys.stderr)
        with open(out, 'wb') as w:
            w.write(b'RIFF' + struct.pack('<I', 36 + len(pcm)) + b'WAVEfmt ' +
                    struct.pack('<IHHIIHH', 16, 1, par['ch'], par['rate'],
                                par['rate'] * par['ch'] * 2, par['ch'] * 2, 16) +
                    b'data' + struct.pack('<I', len(pcm)) + pcm)
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
