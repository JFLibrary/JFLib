//! Complete table transfer, with fresh identities and collision-safe live style graphs.
use super::*;
impl Document {
    /// Copy one complete table, including merged cells, headers, borders, styles and cited sources.
    /// A single empty trailing body paragraph anchors this structural fragment.
    pub fn copy_table(&self,id:Id)->Result<DocumentFragment,Error>{
        self.validate()?;let table=self.table(id).ok_or(Error::Missing)?.clone();
        let mut d=Document::default();d.page=self.page;d.paragraphs[0].id=table.before;
        d.styles=self.styles.clone();d.tables.push(table);
        for child in self.ordered_tables() {
            if d.tables.iter().any(|t| t.cells().any(|c| c.paragraphs.iter().any(|p| p.id == child.before))) { d.tables.push(child.clone()); }
        }
        let mut used=BTreeSet::new();for t in &d.tables {for (_,_,cell) in t.anchors(){for p in &cell.paragraphs{source_ids(&p.content,&mut used);}}}
        let anchors:BTreeSet<_>=d.body_paragraphs().flat_map(|p|&p.content).filter_map(|n|if let Inline::NoteReference(id)=n{Some(*id)}else{None}).collect();
        d.notes=self.notes.iter().filter(|n|anchors.contains(&n.id)).cloned().collect();
        for note in &d.notes { for p in &note.paragraphs { source_ids(&p.content,&mut used); } }
        d.sources=self.sources.iter().filter(|s|used.contains(&s.id)).cloned().collect();
        let mut used_styles=BTreeSet::new();
        for t in &d.tables {let mut style=t.style.clone(); while let Some(id)=style {
            let s=self.table_style(&id).ok_or(Error::Missing)?;style=s.based_on.clone();used_styles.insert(id);
        }}
        // Match the canonical DOCX order; untouched built-ins remain implicit, but the
        // destination importer still compares their definitions before reusing their IDs.
        let defaults=Document::default();
        for id in self.table_style_ids().into_iter().filter(|id|used_styles.contains(id)){
            let s=self.table_style(&id).ok_or(Error::Missing)?;if defaults.table_style(&id).as_ref()!=Some(&s){d.table_styles.push(s);}
        }
        d.validate()?;Ok(DocumentFragment{document:d})
    }
    pub(super) fn insert_table_fragment(&mut self,left:Id,right:Id,mut left_p:Paragraph,right_p:Paragraph,mut prefix:Vec<Inline>,mut suffix:Vec<Inline>,mut incoming:Document,allocate:&mut impl FnMut()->Result<Id,Error>)->Result<Id,Error>{
        if self.note_for_paragraph(left).is_some() { return Err(Error::Invalid); }
        let group = self.paragraph_group(left).ok_or(Error::Missing)?;
        if !group.iter().any(|p| p.id == right) { return Err(Error::Invalid); }
        let (a,b)=self.paragraph_bounds(left);incoming.fit_tables_to_width(b-a)?;
        self.import_fragment_styles(&mut incoming)?;
        self.import_table_fragment_styles(&mut incoming)?;
        let mut sources=BTreeMap::new();
        for s in &mut incoming.sources {
            let old=s.id;let id=if let Some(existing)=self.sources.iter().find(|v|v.same_content(s)){existing.id}else{s.id=allocate()?;let id=s.id;self.sources.push(s.clone());id};sources.insert(old,id);
        }
        let mut notes=BTreeMap::new();
        for note in &incoming.notes { notes.insert(note.id,allocate()?); }
        for mut note in incoming.notes {
            note.id=*notes.get(&note.id).ok_or(Error::Missing)?;
            for p in &mut note.paragraphs {p.id=allocate()?;remap(&mut p.content,&sources,&notes)?;field_carets(&mut p.content);}
            self.notes.push(note);
        }
        let starts_table = incoming.tables.iter().any(|t| t.before == incoming.paragraphs[0].id);
        let separate_prefix = starts_table && prefix.iter().any(|i| i.text()!=Some(""));
        let mut ids=BTreeMap::new();
        for (i,p) in incoming.paragraphs.iter_mut().enumerate() {
            let old=p.id;p.id=if i==0 && !separate_prefix {left}else{allocate()?};ids.insert(old,p.id);
            remap(&mut p.content,&sources,&notes)?;
        }
        for t in &mut incoming.tables {t.id=allocate()?; for row in &mut t.rows {for cell in &mut row.cells {
            cell.id=allocate()?;for p in &mut cell.paragraphs {let old=p.id;p.id=allocate()?;ids.insert(old,p.id);remap(&mut p.content,&sources,&notes)?;field_carets(&mut p.content);}
        }}}
        for t in &mut incoming.tables {t.before=*ids.get(&t.before).ok_or(Error::Missing)?;}
        if separate_prefix {field_carets(&mut prefix);left_p.content=prefix;incoming.paragraphs.insert(0,left_p);}
        else if !starts_table {
            let p=&mut incoming.paragraphs[0];
            if prefix.iter().any(|i|i.text()!=Some("")){preserve_style(self,&mut p.content,p.style,left_p.style);p.style=left_p.style;p.format=left_p.format;}
            prefix.append(&mut p.content);p.content=prefix;
        }
        let p=incoming.paragraphs.last_mut().ok_or(Error::Invalid)?;
        if p.content.iter().all(|i|i.text()==Some("")) {p.content.clear();p.style=right_p.style;p.format=right_p.format;}
        preserve_style(self,&mut suffix,right_p.style,p.style);p.content.extend(suffix);let tail=p.id;
        for p in &mut incoming.paragraphs {crate::text_range::coalesce(&mut p.content);field_carets(&mut p.content);}
        let group=self.paragraph_group_mut(left).ok_or(Error::Missing)?;
        let first=group.iter().position(|p|p.id==left).ok_or(Error::Missing)?;
        let last=group.iter().position(|p|p.id==right).ok_or(Error::Missing)?;
        group.splice(first..=last,incoming.paragraphs);self.tables.extend(incoming.tables);
        self.sort_tables();
        self.prune_notes();Ok(tail)
    }
    fn import_table_fragment_styles(&mut self,incoming:&mut Document)->Result<(),Error>{
        let mut pending=Vec::new();let mut seen=BTreeSet::new();
        for t in &incoming.tables {let mut chain=Vec::new();let mut key=t.style.clone();
            while let Some(id)=key {if !seen.insert(id.clone()){break;}let s=incoming.table_style(&id).ok_or(Error::Missing)?;key=s.based_on.clone();chain.push(s);}
            pending.extend(chain.into_iter().rev());
        }
        let mut mapping=BTreeMap::<String,String>::new();
        for mut s in pending {
            let old=s.id.clone();if let Some(base)=&s.based_on{s.based_on=Some(mapping.get(base).ok_or(Error::Missing)?.clone());}
            if self.table_style(&old).as_ref()!=Some(&s){
                let same=self.table_style_ids().into_iter().find(|id|self.table_style(id).is_some_and(|other|{let mut equivalent=s.clone();equivalent.id=other.id.clone();equivalent.name=other.name.clone();equivalent==other && (other.name==s.name || other.name.starts_with(&alloc::format!("{} (",s.name)))}));
                if let Some(id)=same{s.id=id;}else{
                    let mut n=1;while self.table_style(&alloc::format!("jfTablePaste{n}")).is_some(){n+=1;}s.id=alloc::format!("jfTablePaste{n}");
                    let original=s.name.clone();let mut suffix=2;
                    while self.table_style_ids().iter().any(|id|self.table_style(id).is_some_and(|v|v.name.eq_ignore_ascii_case(&s.name))){
                        let tail=alloc::format!(" ({suffix})");let mut end=original.len().min(128-tail.len());while !original.is_char_boundary(end){end-=1;}
                        s.name=alloc::format!("{}{}",&original[..end],tail);suffix+=1;
                    }
                    self.table_styles.push(s.clone());
                }
            }
            mapping.insert(old,s.id);
        }
        for t in &mut incoming.tables {if let Some(id)=&mut t.style{*id=mapping.get(id).ok_or(Error::Missing)?.clone();}}Ok(())
    }
}
