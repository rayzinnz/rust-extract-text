--sqlite3 "C:\temp\MsgSearcher.db"

--sqlite3 /home/ray/MsgSearcher.db

--update path to generic :
-- UPDATE f SET path = replace(replace(path,'C:\OutlookItems\',''),'\',':');
-- SELECT path, replace(replace(path,'C:\OutlookItems\',''),'\',':') FROM f limit 10;


--move to virtual table with FTS5
CREATE TABLE fbackup (
        fld_rowid INT
        ,filename TEXT
        ,path TEXT
        ,size INT
        ,time INT
        ,contents TEXT
        ,status INT

        ,subject TEXT
        ,sender TEXT
        ,recipient TEXT
        ,cc TEXT
        ,senddate INT
        ,attachments TEXT
);
select datetime();
insert into fbackup select * from f;
select datetime();

drop table f;
VACUUM;

CREATE VIRTUAL TABLE IF NOT EXISTS f USING fts5(
        fld_rowid
        ,filename
        ,path
        ,size
        ,time
        ,contents
        ,status

        ,subject
        ,sender
        ,recipient
        ,cc
        ,senddate
        ,attachments
);

select datetime();
insert into f select * from fbackup;
select datetime();

select datetime();
drop table fbackup;
select datetime();
VACUUM;
select datetime();


SELECT CASE WHEN size = 323584 AND time = 1595441780 THEN 'L' ELSE 'U' END as [refresh_check], ROWID
FROM f
WHERE filename MATCH '"20200722_163937_EUNZAU_Production_monitoring_tool.msg"'
AND path MATCH '"2020:07"'
;

SELECT count(*) -- CASE WHEN size = 323584 AND time = 1595441780 THEN 'L' ELSE 'U' END as [refresh_check], ROWID
FROM f
WHERE f MATCH 'filename:"20200722_163937_EUNZAU_Production_monitoring_tool.msg" AND path:"2020:07"'
;

.schema
CREATE TABLE fld (
        name TEXT
        ,path TEXT
        ,extensions TEXT
        ,include_subfolders INT
);
-- CREATE TABLE f (
--         fld_rowid INT
--         ,filename TEXT
--         ,path TEXT
--         ,size INT
--         ,time INT
--         ,contents TEXT
--         ,status INT

--         ,subject TEXT
--         ,sender TEXT
--         ,recipient TEXT
--         ,cc TEXT
--         ,senddate INT
--         ,attachments TEXT
-- );
-- CREATE INDEX idx_filepath ON f(filename,path);
-- -- CREATE INDEX idx_from ON f(sender);
-- -- CREATE INDEX idx_subject ON f(subject);
-- -- drop INDEX idx_from;
-- -- drop INDEX idx_subject;
-- CREATE INDEX idx_senddate ON f(senddate);
-- CREATE INDEX idx_from_senddate ON f(sender,senddate);

select count(*) from f;

select datetime(f.senddate),f.subject,f.sender,f.recipient,f.cc,f.size,f.path,f.filename
from f
join fld on fld.rowid=f.fld_rowid
where 1=1
and f.subject LIKE '%sprint%'
and f.sender LIKE '%ray%'
--and coalesce(f.recipient,'')||coalesce(f.cc,'') LIKE '%{}%'
--and f.contents glob '*{}*'
order by f.senddate desc
limit 10;

EXPLAIN QUERY PLAN
--select datetime();
select datetime(f.senddate),f.subject,f.sender,f.recipient,f.cc,f.size,f.path,f.filename
from f
join fld on fld.rowid=f.fld_rowid
where 1=1
--and f.sender LIKE '%hwang%'
and f.sender GLOB '*hwang*'
--order by f.senddate desc
limit 30;
select datetime();

-- https://www.sqlite.org/fts5.html#:~:text=FTS5%20is%20an%20SQLite%20virtual,instances%20of%20a%20search%20term

select datetime(f.senddate),f.subject
from f
join fld on fld.rowid=f.fld_rowid
where f MATCH 'subject:("consumable" AND "meat")
                '
--order by f.senddate desc
limit 30;

select datetime(f.senddate),f.subject
from f
join fld on fld.rowid=f.fld_rowid
where f MATCH 'subject:("consumable" "meat")'
--order by f.senddate desc
limit 30;

select datetime(f.senddate),f.subject
from f
join fld on fld.rowid=f.fld_rowid
where f MATCH 'subject:("cons"* "meat") AND sender:("ray"* "g"*)'
--order by f.senddate desc
limit 30;
