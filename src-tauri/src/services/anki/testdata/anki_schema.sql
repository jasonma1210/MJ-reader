-- Anki 21+ 最小表结构（用于单元测试构造 .apkg）
CREATE TABLE col (
    id INTEGER PRIMARY KEY,
    crt INTEGER,
    mod INTEGER,
    scm INTEGER,
    ver INTEGER,
    dty INTEGER,
    usn INTEGER,
    ls INTEGER,
    conf TEXT,
    models TEXT,
    decks TEXT,
    dconf TEXT,
    tags TEXT
);

CREATE TABLE notes (
    id INTEGER PRIMARY KEY,
    guid TEXT,
    mid INTEGER,
    mod INTEGER,
    usn INTEGER,
    tags TEXT,
    flds TEXT,
    sfld TEXT,
    csum INTEGER,
    flags INTEGER,
    data TEXT
);

CREATE TABLE cards (
    id INTEGER PRIMARY KEY,
    nid INTEGER,
    did INTEGER,
    ord INTEGER,
    mod INTEGER,
    usn INTEGER,
    type INTEGER,
    queue INTEGER,
    due INTEGER,
    ivl INTEGER,
    factor INTEGER,
    reps INTEGER,
    lapses INTEGER,
    left INTEGER,
    odue INTEGER,
    odid INTEGER,
    flags INTEGER,
    data TEXT
);

CREATE TABLE revlog (
    id INTEGER PRIMARY KEY,
    cid INTEGER,
    usn INTEGER,
    ease INTEGER,
    ivl INTEGER,
    lastIvl INTEGER,
    factor INTEGER,
    time INTEGER,
    type INTEGER
);

CREATE TABLE graves (
    usn INTEGER,
    oid INTEGER,
    type INTEGER
);
