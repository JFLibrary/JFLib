// store -- what the Akademie reads and writes: its folders, the courses in them, a lesson, a picture,
// the learner's progress.  Only here the Akademie touches files, and only inside a course's folder.
//
//   place(headless, dir) -> Place     where the courses and the progress are: data://academy; with
//                                     --academy-dir PATH that folder; a headless run never looks at
//                                     data:// and keeps its progress in memory
//   list(root) -> Vec<Entry>          every folder with its course, sorted by name: a bad one is an
//                                     entry with its message and hurts nothing else
//   lesson(dir, file, lang) -> (text, lang)   the lesson in that language, else in the other
//   read_quiz(dir, file) -> text      a quiz's or test's file
//   image(dir, addr) -> Option<bytes> a lesson's picture: only what jfn_model::academy::image_file allows
//   read_progress(path) / write_progress(path, &progress)   missing or corrupt: empty
//
// jfn_run::install installs into the same folders, install.rs lists them again; index.json and its checksums
// are their business.  A folder whose name starts with a dot is not a course: the installer fills one and
// renames it when it is complete.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::academy::{self, Course, Problem, Progress, COURSE_FILE, PROGRESS_FILE};

/// the installed courses, the learner's file (doc/VFS.md: data:// needs no permission, a write creates
/// its folders); a headless run's file
const DATA: &str = "data://academy";
const MEMORY: &str = "mem://jfn-academy/progress.json";
/// the largest course.json, lesson and picture that are read (bytes)
const COURSE_MAX: u64 = 1 << 20;
const LESSON_MAX: u64 = 4 << 20;
const IMAGE_MAX: u64 = 16 << 20;
/// the courses that are listed at most (the block of ids of their cards has room for them)
const COURSES_MAX: usize = 512;

/// where the Akademie's files are
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    /// the folder of the courses (None: there are none)
    pub courses: Option<String>,
    pub progress: String,
}

/// where the files are for this run; `dir`: --academy-dir
pub(crate) fn place(headless: bool, dir: Option<String>) -> Place {
    let dir = dir.map(|d| String::from(d.replace('\\', "/").trim_end_matches('/')));
    let progress = match (&dir, headless) {
        (_, true) => String::from(MEMORY),
        (Some(d), false) => format!("{}/{}", d, PROGRESS_FILE),
        (None, false) => format!("{}/{}", DATA, PROGRESS_FILE),
    };
    let courses = match dir {
        Some(d) => Some(d),
        None if headless => None,
        None => Some(String::from(DATA)),
    };
    Place { courses, progress }
}

/// a folder of the courses
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// its name
    pub id: String,
    pub dir: String,
    /// None: its course.json cannot be read
    pub course: Option<Course>,
    pub problems: Vec<Problem>,
    /// the bytes of its files together
    pub bytes: u64,
}

impl Entry {
    /// the course, if nothing keeps it from being offered
    pub(crate) fn usable(&self) -> Option<&Course> {
        self.course.as_ref().filter(|_| !self.problems.iter().any(|p| p.fatal))
    }

    /// its title in that language, for a course that cannot be read its folder's name
    pub(crate) fn title(&self, en: bool) -> &str {
        match &self.course {
            Some(c) if !c.title.is_empty() => c.title.get(en),
            _ => &self.id,
        }
    }
}

// the text of a file that is not too large; a byte order mark is not part of it
fn read_text(path: &str, max: u64) -> Option<String> {
    if plat::fs::stat(path).is_some_and(|m| m.size > max) {
        return None;
    }
    let bytes = plat::read_file(path)?;
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned();
    Some(text)
}

/// the folder `id` below `root`
pub(crate) fn entry(root: &str, id: &str) -> Entry {
    let dir = format!("{}/{}", root, id);
    let text = read_text(&format!("{}/{}", dir, COURSE_FILE), COURSE_MAX);
    let (course, problems) = academy::load(id, text.as_deref(), &|rel| plat::file_exists(&format!("{}/{}", dir, rel)) == 1);
    let bytes = tree_bytes(&dir);
    Entry { id: String::from(id), dir, course, problems, bytes }
}

// the bytes of the files below a folder; links are no files of a course
fn tree_bytes(dir: &str) -> u64 {
    let mut sum = 0u64;
    for e in plat::fs::read_dir(dir).unwrap_or_default() {
        let path = format!("{}/{}", dir, e.name);
        sum = sum.saturating_add(match e.kind {
            plat::fs::FileKind::Dir => tree_bytes(&path),
            plat::fs::FileKind::File => plat::fs::stat(&path).map_or(0, |m| m.size),
            _ => 0,
        });
    }
    sum
}

/// the courses below `root`, by the names of their folders
pub(crate) fn list(root: &str) -> Vec<Entry> {
    let mut names: Vec<String> = plat::fs::read_dir(root).unwrap_or_default().into_iter().filter(|e| e.kind == plat::fs::FileKind::Dir && !e.name.starts_with('.')).map(|e| e.name).collect();
    names.sort();
    names.truncate(COURSES_MAX);
    names.iter().map(|id| entry(root, id)).collect()
}

/// a lesson's text in `lang`, else in the other language, with the language it is in
pub(crate) fn lesson(dir: &str, file: &str, lang: &str) -> Option<(String, &'static str)> {
    academy::lesson_files(file, lang).into_iter().find_map(|(path, l)| read_text(&format!("{}/{}", dir, path), LESSON_MAX).map(|t| (t, l)))
}

/// the text of a quiz's or test's file (the path was checked with the course)
pub(crate) fn read_quiz(dir: &str, file: &str) -> Option<String> {
    read_text(&format!("{}/{}", dir, file), COURSE_MAX)
}

/// the bytes of a lesson's picture: the view asks with the address it made from the course's folder
/// and the text's source (st.images = MI_APP), so this decides what is read
pub(crate) fn image(dir: &str, addr: &str) -> Option<Vec<u8>> {
    let base = format!("{}/", dir);
    let rel = academy::image_file(&base, addr)?;
    let path = format!("{}{}", base, rel);
    if plat::fs::stat(&path).is_some_and(|m| m.size > IMAGE_MAX) {
        return None;
    }
    plat::read_file(&path)
}

/// the learner's file; missing or corrupt: nothing done yet
pub(crate) fn read_progress(path: &str) -> Progress {
    read_text(path, COURSE_MAX).map(|t| Progress::from_text(&t)).unwrap_or_default()
}

/// the learner's file -> it was written
pub(crate) fn write_progress(path: &str, p: &Progress) -> bool {
    plat::write_file(path, p.to_text().as_bytes())
}
