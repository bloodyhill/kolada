"""Makes the documents the converter's tests read. Run from this folder with
a Python that has python-docx, fpdf2 and pikepdf:

    python make.py

(`python make.py turned_and_locked invoice` makes only those samples.) All content is made up here. Each document holds the same note, so the
tests can expect the same Markdown from each (as near as the format allows):

    # Launch plan
    Ship the **public beta** on *October 12*, see [the post](https://notsy.app/beta).
    ## What ships
    - Live Markdown
    - Pictures
      - pasted
      - dropped
    1. Write the notes
    2. Record the video
    | Item | Owner | Due |
    ...
"""
import os
import struct
import zlib
import subprocess

here = os.path.dirname(os.path.abspath(__file__))
out = lambda name: os.path.join(here, name)


def png(width, height, rgb):
    """A small solid PNG."""
    raw = b''.join(b'\x00' + bytes(rgb) * width for _ in range(height))
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xFFFFFFFF)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')


ROWS = [('Item', 'Owner', 'Due'), ('Press kit', 'Sam', 'Oct 5'), ('Pricing page', 'Lena', 'Oct 8')]
ARABIC = '\u0645\u0644\u0627\u062d\u0638\u0629 \u0642\u0635\u064a\u0631\u0629 \u0628\u0627\u0644\u0639\u0631\u0628\u064a\u0629'


def docx():
    from docx import Document
    from docx.oxml.ns import qn
    from docx.oxml import OxmlElement
    from docx.shared import Inches
    import io
    d = Document()
    d.core_properties.title = 'Launch plan'
    d.add_heading('Launch plan', level=1)
    p = d.add_paragraph('Ship the ')
    p.add_run('public beta').bold = True
    p.add_run(' on ')
    p.add_run('October 12').italic = True
    p.add_run(', see ')
    # A hyperlink: a w:hyperlink with a relationship to the address.
    rid = d.part.relate_to('https://notsy.app/beta', 'http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink', is_external=True)
    link = OxmlElement('w:hyperlink')
    link.set(qn('r:id'), rid)
    run = OxmlElement('w:r')
    text = OxmlElement('w:t')
    text.text = 'the post'
    run.append(text)
    link.append(run)
    p._p.append(link)
    p.add_run('.')
    d.add_heading('What ships', level=2)
    d.add_paragraph('Live Markdown', style='List Bullet')
    d.add_paragraph('Pictures', style='List Bullet')
    d.add_paragraph('pasted', style='List Bullet 2')
    d.add_paragraph('dropped', style='List Bullet 2')
    d.add_paragraph('Write the notes', style='List Number')
    d.add_paragraph('Record the video', style='List Number')
    t = d.add_table(rows=3, cols=3)
    for r, row in enumerate(ROWS):
        for c, value in enumerate(row):
            t.cell(r, c).text = value
    d.add_paragraph('A picture of the cover:')
    d.add_picture(io.BytesIO(png(24, 16, (70, 60, 170))), width=Inches(1.0))
    d.add_paragraph(ARABIC)
    q = d.add_paragraph('Tabs\tand line')
    q.add_run().add_break()
    q.add_run('breaks stay.')
    d.save(out('sample.docx'))


def pdfs():
    from fpdf import FPDF
    # Windows' own fonts, embedded as Type0 with a ToUnicode map, as most
    # PDFs made today have them.
    pdf = FPDF()
    pdf.set_title('Launch plan')
    pdf.add_page()
    pdf.add_font('Segoe', '', r'C:\Windows\Fonts\segoeui.ttf')
    pdf.add_font('Segoe', 'B', r'C:\Windows\Fonts\segoeuib.ttf')
    pdf.add_font('Segoe', 'I', r'C:\Windows\Fonts\segoeuii.ttf')
    pdf.set_font('Segoe', 'B', 24)
    pdf.cell(0, 14, 'Launch plan', new_x='LMARGIN', new_y='NEXT')
    pdf.set_font('Segoe', '', 11)
    pdf.multi_cell(0, 6, 'Ship the **public beta** on __October 12__, see the post at https://notsy.app/beta. '
                   'This paragraph is long enough to wrap onto a second line on the page, so the converter has to join '
                   'the lines it was broken into back into one paragraph.', markdown=True, new_x='LMARGIN', new_y='NEXT')
    pdf.ln(4)
    pdf.set_font('Segoe', 'B', 16)
    pdf.cell(0, 10, 'What ships', new_x='LMARGIN', new_y='NEXT')
    pdf.set_font('Segoe', '', 11)
    for line in ['\u2022 Live Markdown', '\u2022 Pictures', '1. Write the notes', '2. Record the video']:
        pdf.cell(0, 7, line, new_x='LMARGIN', new_y='NEXT')
    pdf.ln(4)
    pdf.set_font('Segoe', 'B', 16)
    pdf.cell(0, 10, 'Second page follows', new_x='LMARGIN', new_y='NEXT')
    pdf.add_page()
    pdf.set_font('Segoe', '', 11)
    pdf.multi_cell(0, 6, 'The second page has its own paragraph, and the converter keeps the pages in order.', new_x='LMARGIN', new_y='NEXT')
    pdf.output(out('sample-embedded.pdf'))

    # The 14 standard fonts, not embedded, WinAnsi encoded.
    pdf = FPDF()
    pdf.add_page()
    pdf.set_font('Helvetica', 'B', 22)
    pdf.cell(0, 12, 'Plain fonts', new_x='LMARGIN', new_y='NEXT')
    pdf.set_font('Helvetica', '', 11)
    pdf.multi_cell(0, 6, 'Caf\u00e9 menus cost 5 euros, a test of WinAnsi encoding with "quotes" and an \u00fcmlaut.', new_x='LMARGIN', new_y='NEXT')
    pdf.output(out('sample-standard.pdf'))

    # The embedded one again, its objects packed into object streams with a
    # cross-reference stream, as PDF 1.5 and later writers do.
    import pikepdf
    with pikepdf.open(out('sample-embedded.pdf')) as p:
        p.save(out('sample-objstm.pdf'), object_stream_mode=pikepdf.ObjectStreamMode.generate, compress_streams=True)
    turned_and_locked()
    invoice()


def invoice():
    """An invoice as billing tools set one: headings set left over amounts
    set right, so no column's words start in line, and rows padded apart."""
    from fpdf import FPDF
    pdf = FPDF()
    pdf.add_page()
    pdf.set_font('Helvetica', 'B', 18)
    pdf.cell(0, 12, 'Invoice 1042', new_x='LMARGIN', new_y='NEXT')
    pdf.ln(6)
    widths = [70, 30, 40, 40]
    rows = [['Item', 'Qty', 'Rate', 'Amount'], ['Design review', '2', '150.00', '300.00'], ['Launch video', '1', '1,200.00', '1,200.00']]
    for i, row in enumerate(rows):
        pdf.set_font('Helvetica', 'B' if i == 0 else '', 10)
        for k, (w, text) in enumerate(zip(widths, row)):
            pdf.cell(w, 12, text, align='L' if i == 0 or k == 0 else 'R')
        pdf.ln(12)
    pdf.ln(6)
    pdf.set_font('Helvetica', '', 10)
    pdf.multi_cell(0, 6, 'Paid within 30 days of the date above.', new_x='LMARGIN', new_y='NEXT')
    pdf.output(out('sample-invoice.pdf'))


def turned_and_locked():
    """The standard-fonts PDF locked against copying, as many PDFs are,
    though they open without a password: with RC4 (revision 3), AES-128
    (revision 4) and AES-256 (revision 6, which Notsy does not read yet).
    And its page drawn sideways on a landscape box, set upright by
    /Rotate 90 as some writers and scanners do."""
    import pikepdf
    no_copying = pikepdf.Permissions(extract=False)
    for name, revision, aes in [('rc4', 3, False), ('aes128', 4, True), ('aes256', 6, True)]:
        with pikepdf.open(out('sample-standard.pdf')) as p:
            p.save(out('sample-%s.pdf' % name), encryption=pikepdf.Encryption(owner='owner', user='', R=revision, aes=aes, metadata=revision >= 4, allow=no_copying))
    with pikepdf.open(out('sample-standard.pdf')) as p:
        page = p.pages[0]
        w, h = float(page.MediaBox[2]), float(page.MediaBox[3])
        page.contents_add(p.make_stream(b'q 0 1 -1 0 %g 0 cm' % h), prepend=True)
        page.contents_add(p.make_stream(b'Q'))
        page.MediaBox = [0, 0, h, w]
        page.Rotate = 90
        p.save(out('sample-rotated.pdf'))


HTML = '''<!doctype html>
<html><head><meta charset="utf-8"><title>Launch plan</title>
<style>body { font: 14px sans-serif } .nav { color: red }</style>
<script>window.tracking = 1; document.write("<p>never</p>");</script></head>
<body>
<nav class="nav"><a href="/">Home</a> | <a href="/blog">Blog</a></nav>
<article>
<h1>Launch plan</h1>
<p>Ship the <strong>public beta</strong> on <em>October 12</em>, see
   <a href="https://notsy.app/beta" title="Beta">the post</a>.</p>
<h2>What ships</h2>
<ul><li>Live Markdown</li><li>Pictures<ul><li>pasted</li><li>dropped</li></ul></li></ul>
<ol><li>Write the notes</li><li>Record the video</li></ol>
<table><thead><tr><th>Item</th><th>Owner</th><th>Due</th></tr></thead>
<tbody><tr><td>Press kit</td><td>Sam</td><td>Oct 5</td></tr><tr><td>Pricing page</td><td>Lena</td><td>Oct 8</td></tr></tbody></table>
<p>A picture of the cover: <img src="cover.png" alt="Cover"></p>
<blockquote><p>Speed is a feature.</p></blockquote>
<pre><code>cargo build --release
notsy --trace</code></pre>
<p>Entities: &amp; &lt;tags&gt; &nbsp;&copy; &#8364; &#x2014; and a line<br>break.</p>
<p dir="rtl">''' + ARABIC + '''</p>
</article>
<footer>Footer text that is still text.</footer>
</body></html>
'''


def html():
    with open(out('sample.html'), 'w', encoding='utf-8', newline='\n') as f:
        f.write(HTML)
    with open(out('cover.png'), 'wb') as f:
        f.write(png(24, 16, (70, 60, 170)))
    # Chromium's own PDF of the page: Skia's fonts and layout.
    edge = r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
    if os.path.exists(edge):
        url = 'file:///' + out('sample.html').replace('\\', '/')
        subprocess.run([edge, '--headless', '--disable-gpu', '--no-pdf-header-footer', '--print-to-pdf=' + out('sample-chromium.pdf'), url],
                       check=True, timeout=60, capture_output=True)


import sys
if sys.argv[1:]:
    # Only the samples named, the others as they are.
    for name in sys.argv[1:]:
        globals()[name]()
else:
    docx()
    pdfs()
    html()
print('made', sorted(f for f in os.listdir(here) if not f.endswith('.py')))
