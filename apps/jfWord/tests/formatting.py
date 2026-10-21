"""Independent OOXML assertions and supported/unsupported formatting input cases."""
import xml.etree.ElementTree as ET
import zipfile

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'

def run(invoke,rel,out,parts,zip_parts,root):
    rich=root/'build/jfword-editor.docx'
    with zipfile.ZipFile(rich) as z:
        doc=ET.fromstring(z.read('word/document.xml'))
    assert doc.find('.//'+W+'b') is not None
    assert doc.find('.//'+W+'i') is not None
    assert doc.find('.//'+W+'u').get(W+'val')=='single'
    assert doc.find('.//'+W+'jc').get(W+'val')=='both'
    assert doc.find('.//'+W+'spacing').get(W+'line')=='360'
    assert len(doc.findall('.//'+W+'pageBreakBefore'))==1
    assert len(doc.findall('.//'+W+'fldSimple'))==4
    roundtrip=out/'editor-roundtrip.docx'
    invoke('--check-docx',rel(rich),'--output',rel(roundtrip))
    assert rich.read_bytes()==roundtrip.read_bytes()
    paragraph=b'<w:pPr><w:pStyle w:val="Heading1"/></w:pPr>'
    variants=[
        ('bold',b'<w:r>',b'<w:r><w:rPr><w:b/></w:rPr>',True),
        ('rgb-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="aBcD09"/></w:rPr>',True),
        ('automatic-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="auto"/></w:rPr>',True),
        ('strike',b'<w:r>',b'<w:r><w:rPr><w:strike/></w:rPr>',True),
        ('explicit-no-strike',b'<w:r>',b'<w:r><w:rPr><w:strike w:val="off"/></w:rPr>',True),
        ('optional-main-paragraph-id',b' w14:paraId="00000001"',b'',True),
        ('optional-markup-compatibility',b' mc:Ignorable="w14"',b'',True),
        ('invalid-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="12345G"/></w:rPr>',False),
        ('alpha-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="FF123456"/></w:rPr>',False),
        ('missing-color',b'<w:r>',b'<w:r><w:rPr><w:color/></w:rPr>',False),
        ('theme-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="123456" w:themeColor="accent1"/></w:rPr>',False),
        ('double-strike',b'<w:r>',b'<w:r><w:rPr><w:strike/><w:dstrike/></w:rPr>',False),
        ('explicit-not-bold',b'<w:r>',b'<w:r><w:rPr><w:b w:val="false"/><w:i w:val="on"/><w:u w:val="none"/><w:sz w:val="27"/></w:rPr>',True),
        ('paragraph-properties',paragraph,b'<w:pPr><w:pStyle w:val="Heading1"/><w:pageBreakBefore w:val="true"/><w:spacing w:before="160" w:after="240" w:line="480" w:lineRule="auto"/><w:ind w:left="360" w:right="480"/><w:jc w:val="center"/></w:pPr>',True),
        ('landscape',b'<w:pgSz w:w="11906" w:h="16838"/>',b'<w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>',True),
        ('duplicate-run-property',b'<w:r>',b'<w:r><w:rPr><w:b/><w:b/></w:rPr>',False),
        ('bad-run-size',b'<w:r>',b'<w:r><w:rPr><w:sz w:val="65535"/></w:rPr>',False),
        ('unknown-underline',b'<w:r>',b'<w:r><w:rPr><w:u w:val="wave"/></w:rPr>',False),
        ('bad-onoff',b'<w:r>',b'<w:r><w:rPr><w:b w:val="maybe"/></w:rPr>',False),
        ('wrong-property-namespace',b'<w:r>',b'<w:r><w:rPr><b xmlns="urn:wrong"/></w:rPr>',False),
        ('nested-property',b'<w:r>',b'<w:r><w:rPr><w:b><w:i/></w:b></w:rPr>',False),
        ('oversized-indent',paragraph,b'<w:pPr><w:ind w:left="65000"/></w:pPr>',False),
        ('unknown-line-rule',paragraph,b'<w:pPr><w:spacing w:line="240" w:lineRule="exact"/></w:pPr>',False),
        ('duplicate-p-property',paragraph,b'<w:pPr><w:jc w:val="left"/><w:jc w:val="right"/></w:pPr>',False),
        ('firstline',paragraph,b'<w:pPr><w:ind w:firstLine="360"/></w:pPr>',True),
        ('hanging',paragraph,b'<w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>',True),
        ('hanging-in-margin',paragraph,b'<w:pPr><w:ind w:hanging="360"/></w:pPr>',True),
        ('zero-firstline',paragraph,b'<w:pPr><w:ind w:firstLine="0"/></w:pPr>',True),
        ('zero-hanging',paragraph,b'<w:pPr><w:ind w:hanging="0"/></w:pPr>',True),
        ('negative-firstline',paragraph,b'<w:pPr><w:ind w:firstLine="-360"/></w:pPr>',False),
        ('negative-hanging',paragraph,b'<w:pPr><w:ind w:hanging="-360"/></w:pPr>',False),
        ('firstline-outside-text',paragraph,b'<w:pPr><w:ind w:firstLine="9000"/></w:pPr>',False),
        ('hanging-outside-paper',paragraph,b'<w:pPr><w:ind w:hanging="1418"/></w:pPr>',False),
        ('overlapping-firstline-hanging',paragraph,b'<w:pPr><w:ind w:firstLine="360" w:hanging="720"/></w:pPr>',False),
        ('unsupported-character-indent',paragraph,b'<w:pPr><w:ind w:firstLine="360" w:firstLineChars="200"/></w:pPr>',False),
        ('empty-page',b'w:left="1417"',b'w:left="11906"',False),
        ('invalid-page-orientation',b'w:h="16838"',b'w:h="16838" w:orient="landscape"',False),
    ]
    for name,old,new,valid in variants:
        edited=dict(parts)
        assert old in edited['word/document.xml'],name
        edited['word/document.xml']=edited['word/document.xml'].replace(old,new,1)
        path=out/('format-'+name+'.docx');path.write_bytes(zip_parts(edited))
        args=['--check-docx',rel(path)]
        if not valid:args.append('--expect-reject')
        invoke(*args)
        if name in ('firstline', 'hanging', 'hanging-in-margin'):
            saved = out / ('format-' + name + '-saved.docx')
            invoke('--check-docx', rel(path), '--output', rel(saved))
            with zipfile.ZipFile(saved) as archive:
                saved_doc = ET.fromstring(archive.read('word/document.xml'))
            indent = saved_doc.find('.//' + W + 'ind')
            attribute = 'firstLine' if name == 'firstline' else 'hanging'
            assert indent.get(W + attribute) == '360'
            assert indent.get(W + ('hanging' if attribute == 'firstLine' else 'firstLine')) is None
            # Unspecified properties inherit; these fixtures have the Word default of zero.
            assert indent.get(W + 'left', '0') == ('720' if name == 'hanging' else '0')
    for theme,scale in [('light','100'),('dark','150')]:
        invoke('--file',rel(rich),'--preview','--no-sources','--theme',theme,'--scale',scale,
               '--shot',rel(out/f'editor-preview-{theme}-{scale}.png'),'--verify-dirty')
