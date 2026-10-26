-- OpenCode 2.0.32: the session_v2 and session_message tables of a throwaway database.
CREATE TABLE `session_v2` (
          `id` text PRIMARY KEY,
          `project_id` text NOT NULL,
          `workspace_id` text,
          `parent_id` text,
          `fork_session_id` text,
          `fork_boundary` text,
          `slug` text NOT NULL,
          `directory` text NOT NULL,
          `path` text,
          `title` text,
          `version` text NOT NULL,
          `share_url` text,
          `summary_additions` integer,
          `summary_deletions` integer,
          `summary_files` integer,
          `summary_diffs` text,
          `metadata` text,
          `cost` real DEFAULT 0 NOT NULL,
          `tokens_input` integer DEFAULT 0 NOT NULL,
          `tokens_output` integer DEFAULT 0 NOT NULL,
          `tokens_reasoning` integer DEFAULT 0 NOT NULL,
          `tokens_cache_read` integer DEFAULT 0 NOT NULL,
          `tokens_cache_write` integer DEFAULT 0 NOT NULL,
          `revert` text,
          `permission` text,
          `agent` text,
          `model` text,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `time_idle` integer,
          `time_viewed` integer,
          `idle_outcome` text,
          `time_compacting` integer,
          `time_archived` integer,
          `time_suspended` integer,
          `resume_attempts` integer DEFAULT 0 NOT NULL,
          CONSTRAINT `fk_session_v2_project_id_project_id_fk` FOREIGN KEY (`project_id`) REFERENCES `project`(`id`) ON DELETE CASCADE
        );
CREATE INDEX `session_v2_project_idx` ON `session_v2` (`project_id`);
CREATE INDEX `session_v2_workspace_idx` ON `session_v2` (`workspace_id`);
CREATE INDEX `session_v2_parent_idx` ON `session_v2` (`parent_id`);
CREATE INDEX `session_v2_time_suspended_idx` ON `session_v2` (`time_suspended`) WHERE "session_v2"."time_suspended" is not null;
CREATE TABLE `session_message` (
          `id` text PRIMARY KEY,
          `session_id` text NOT NULL,
          `type` text NOT NULL,
          `seq` integer NOT NULL,
          `time_created` integer NOT NULL,
          `time_updated` integer NOT NULL,
          `data` text NOT NULL,
          CONSTRAINT `fk_session_message_session_id_session_v2_id_fk` FOREIGN KEY (`session_id`) REFERENCES `session_v2`(`id`) ON DELETE CASCADE
        );
CREATE UNIQUE INDEX `session_message_session_seq_idx` ON `session_message` (`session_id`,`seq`);
CREATE INDEX `session_message_session_type_seq_idx` ON `session_message` (`session_id`,`type`,`seq`);
CREATE INDEX `session_message_session_time_created_id_idx` ON `session_message` (`session_id`,`time_created`,`id`);
CREATE INDEX `session_message_time_created_idx` ON `session_message` (`time_created`);
INSERT INTO session_v2 VALUES('ses_fixture0000000000000001','global',NULL,NULL,NULL,NULL,'hidden-cactus','/tmp/yardsort-fixture/repo-opencode','','One-sentence summary of notes.txt','2.0.32',NULL,NULL,NULL,NULL,NULL,NULL,0.0,5047,128,251,27200,0,NULL,NULL,NULL,'{"id":"glm-5.3-flash-free","providerID":"opencode"}',1793005183486,1793005194164,1793005194168,NULL,'succeeded',NULL,NULL,NULL,0);
INSERT INTO session_message VALUES('msg_1776fe203001I8S9uLiDK1AHqt','ses_fixture0000000000000001','user',4,1793005183668,1793005183669,'{"time":{"created":1793005183668},"text":"\"Read notes.txt, then reply with one short sentence saying what it says.\"","files":[]}');
INSERT INTO session_message VALUES('msg_1776fe2ce0019U4x4k22bJjZmu','ses_fixture0000000000000001','assistant',5,1793005183714,1793005185692,'{"time":{"created":1793005183714,"streamed":1793005185675,"completed":1793005185690},"agent":"build","model":{"id":"glm-5.3-flash-free","providerID":"opencode","variant":"default"},"content":[{"type":"reasoning","text":"The user wants me to read notes.txt and reply with one short sentence about what it says. Simple task — I just need to read the file and summarize its contents briefly.","state":{"reasoningField":"reasoning_content"},"time":{"created":1793005185201,"completed":1793005185667}},{"type":"tool","id":"chatcmpl-tool-a9da03cbc3d013ba","name":"read","executed":false,"state":{"status":"completed","input":{"path":"notes.txt"},"content":[{"type":"text","text":"Read file notes.txt, lines 1-1\n1: the tests need TZ=UTC"}],"metadata":{"truncated":false}},"time":{"created":1793005185662,"ran":1793005185673,"completed":1793005185683}}],"snapshot":{"start":"37653d4e2ed426ef28fd5d3265c288266c46fd96","end":"37653d4e2ed426ef28fd5d3265c288266c46fd96","files":[]},"finish":"tool-calls","rawFinish":"tool_calls","cost":0,"tokens":{"input":3467,"output":12,"reasoning":35,"cache":{"read":4352,"write":0}}}');
INSERT INTO session_message VALUES('msg_1776feaa70011deJyuUg0A9CJW','ses_fixture0000000000000001','assistant',14,1793005185713,1793005187016,'{"time":{"created":1793005185713,"streamed":1793005187006,"completed":1793005187014},"agent":"build","model":{"id":"glm-5.3-flash-free","providerID":"opencode","variant":"default"},"content":[{"type":"text","text":"notes.txt says the tests need to be run with `TZ=UTC`."}],"snapshot":{"start":"37653d4e2ed426ef28fd5d3265c288266c46fd96","end":"37653d4e2ed426ef28fd5d3265c288266c46fd96","files":[]},"finish":"stop","rawFinish":"stop","cost":0,"tokens":{"input":273,"output":17,"reasoning":0,"cache":{"read":7616,"write":0}}}');
INSERT INTO session_message VALUES('msg_1776fefca001qJ9QpHkwyeBKb1','ses_fixture0000000000000001','idle',19,1793005187018,1793005187019,'{"time":{"created":1793005187018},"outcome":"succeeded"}');
INSERT INTO session_message VALUES('msg_1776ff2e4001f9gZ4b8GAGEatK','ses_fixture0000000000000001','user',22,1793005188034,1793005188035,'{"time":{"created":1793005188034},"text":"\"Thanks. In one sentence, what is left to do?\"","files":[]}');
INSERT INTO session_message VALUES('msg_1776ff3da001FaghAmMWV41zox','ses_fixture0000000000000001','assistant',23,1793005188073,1793005191451,'{"time":{"created":1793005188073,"streamed":1793005191392,"completed":1793005191449},"agent":"build","model":{"id":"glm-5.3-flash-free","providerID":"opencode","variant":"default"},"content":[{"type":"reasoning","text":"The user asked \"in one sentence, what is left to do?\" — this likely refers to remaining work in the repo. I should check for TODOs, or maybe there''s a plan/notes. The only context I have is notes.txt which said \"the tests need TZ=UTC\". Maybe there''s something in the repo indicating remaining work. Let me look quickly — check git status, TODOs, or a task list.","state":{"reasoningField":"reasoning_content"},"time":{"created":1793005189552,"completed":1793005191380}},{"type":"tool","id":"chatcmpl-tool-878d7cb378c5b15a","name":"shell","executed":false,"state":{"status":"completed","input":{"command":"git status --short && git log --oneline -5"},"content":[{"type":"text","text":"?? notes.txt\nfatal: your current branch ''master'' does not have any commits yet\n"},{"type":"text","text":"Exited with code 128"}],"metadata":{"status":"completed","truncated":false,"exit":128}},"time":{"created":1793005191004,"ran":1793005191385,"completed":1793005191438}},{"type":"tool","id":"chatcmpl-tool-b1abc88d57f480a5","name":"grep","executed":false,"state":{"status":"completed","input":{"pattern":"TODO|FIXME|left to do|remaining","limit":20},"content":[{"type":"text","text":"No matches found"}],"metadata":{"matches":0,"truncated":false}},"time":{"created":1793005191149,"ran":1793005191390,"completed":1793005191417}}],"snapshot":{"start":"37653d4e2ed426ef28fd5d3265c288266c46fd96","end":"37653d4e2ed426ef28fd5d3265c288266c46fd96","files":[]},"finish":"tool-calls","rawFinish":"tool_calls","cost":0,"tokens":{"input":305,"output":45,"reasoning":87,"cache":{"read":7616,"write":0}}}');
INSERT INTO session_message VALUES('msg_177700127001wkHqfOJ5HJ8RE7','ses_fixture0000000000000001','assistant',38,1793005191470,1793005194166,'{"time":{"created":1793005191470,"streamed":1793005194156,"completed":1793005194164},"agent":"build","model":{"id":"glm-5.3-flash-free","providerID":"opencode","variant":"default"},"content":[{"type":"reasoning","text":"The repo is empty — no commits, only notes.txt untracked. So nothing has been done yet. The only identified task is from notes.txt: run tests with TZ=UTC. But what''s \"left to do\"? The repo is empty except notes.txt, so nothing has been started. I should answer in one sentence based on what I can see.","state":{"reasoningField":"reasoning_content"},"time":{"created":1793005192755,"completed":1793005194150}},{"type":"text","text":"Based on the repo (no commits yet, just notes.txt), everything is still to do: the code needs to be written so its tests can pass with `TZ=UTC`, as noted in `notes.txt`."}],"snapshot":{"start":"37653d4e2ed426ef28fd5d3265c288266c46fd96","end":"37653d4e2ed426ef28fd5d3265c288266c46fd96","files":[]},"finish":"stop","rawFinish":"stop","cost":0,"tokens":{"input":470,"output":45,"reasoning":72,"cache":{"read":7616,"write":0}}}');
INSERT INTO session_message VALUES('msg_177700bb800194s0nSiY32LAzV','ses_fixture0000000000000001','idle',45,1793005194168,1793005194168,'{"time":{"created":1793005194168},"outcome":"succeeded"}');
