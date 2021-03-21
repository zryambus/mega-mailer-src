use rustls_connector::{RustlsConnector, TlsStream};
use std::net::TcpStream;
use imap;
use lazy_static::lazy_static;
use log::{error, debug};
use std::ops::Deref;
use std::iter::FromIterator;
use schedule;
use tokio;

use crate::types::Result;
use crate::storage::{Storage, MailAccount};
use crate::cfg::CONFIG;
use crate::bot::TelegramBot;


lazy_static!{
    static ref STORAGE: Storage = Storage::new().unwrap();
}

pub struct Checker {
    host: String,
    port: u16
}

impl Checker {
    fn new() -> Result<Checker> {
        let host = CONFIG.get::<String>("mail.address");
        let port = CONFIG.get::<u16>("mail.port");

        Ok(
            Checker{
                host,
                port
            }
        )
    }

    fn build_stream(&self) -> Result<TlsStream<TcpStream>> {
        let connector = RustlsConnector::new_with_native_certs()?;
        let stream = TcpStream::connect((self.host.clone(), self.port))?;
        let tls_stream = connector.connect(&self.host, stream)?;
        Ok(tls_stream)
    }

    fn build_client(&self) -> Result<imap::Client<TlsStream<TcpStream>>> {
        let stream = self.build_stream()?;
        Ok(imap::Client::new(stream))
    }

    fn process_message(
        message: &imap::types::Fetch,
        bot: &TelegramBot,
        username: &String,
    ) {
        let envelope = message.envelope().unwrap();
        let from_addr = &envelope.from.as_ref().unwrap()[0];
        let from = if let Some(from) = from_addr.name {
            Some(String::from_utf8_lossy(from))
        } else {
            None
        };

        let subject = if let Some(subject) = envelope.subject.as_ref() {
            Some(String::from_utf8_lossy(subject))
        } else {
            None
        };

        let address = &envelope.from.as_ref().unwrap()[0];
        let host = String::from_utf8_lossy(address.host.unwrap());
        let mailbox = String::from_utf8_lossy(address.mailbox.unwrap());
        let email = format!("{}@{}", mailbox, host);

        let subject = subject.unwrap_or("No subject".into());
        let notify = if from.is_some() {
            format!("*{}*\n{}\n{}", from.unwrap(), email, subject)
        } else {
            format!("*{}*\n{}", email, subject)
        };

        let bot = bot.clone();
        let username = username.clone();
        tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let res = bot.send_markdown(&username, &notify).await;
            if res.is_err() {
                error!(target: "TelegramBot", "{}", res.unwrap_err());
            }
        });
    }

    fn process_account(
        username: &String,
        account: &MailAccount,
        bot: &TelegramBot
    ) {
        let MailAccount {email, password} = account;
        let client = Checker::new().unwrap().build_client().unwrap();
        let mut session = match client.login(email, password).map_err(|e| e.0) {
            Ok(session) => session,
            Err(e) => {
                error!("Could not login into {}: {}", email, e);
                return;
            }
        };

        let folders = session.list(None, Some("INBOX*")).unwrap();
        for folder in folders.iter() {
            let _mailbox = session.select(folder.name()).unwrap();
            let unseen = session.search("UNSEEN").unwrap();

            if unseen.len() == 0 {
                continue;
            }

            let available_uids = Vec::from_iter(unseen.iter());
            let to_fetch_uids = STORAGE.filter_unprocessed(username, available_uids.as_slice()).unwrap();

            let to_fetch = format!("{}", Vec::from_iter(to_fetch_uids.iter().map(|x| x.to_string())).join(","));
            debug!("User: \"{}\" To fetch {}", username, to_fetch);

            let fetched = session.fetch(to_fetch, "ENVELOPE").unwrap();
            for message in fetched.iter() {
                Checker::process_message(message, bot, username);
            }

            STORAGE.add_processed_mails(username, to_fetch_uids.as_slice()).unwrap();
        }

        session.logout().unwrap()
    }

    fn check_on_cron() {
        let bot = TelegramBot::new(STORAGE.deref().clone());
        let users = STORAGE.get_usernames_for_checking();
        if let Ok(users) = &users {
            for user in users {
                let account = STORAGE.get_mail_account(user);
                if account.is_none() {
                    error!(target: "MailChecker", "There is no valid mail account for user {}", user);
                    STORAGE.disable_checking(user).unwrap();
                    continue;
                }

                let account = account.unwrap();
                Checker::process_account(user, &account, &bot);
            }

        } else {
            error!(target: "MailChecker", "{}", users.unwrap_err());
        }
    }

    pub fn start() -> Result<()> {
        let mut agenda = schedule::Agenda::new();

        agenda.add(move ||{
            Checker::check_on_cron();
        }).schedule("0 * * * * *")?;

        std::thread::spawn(move || {
            loop {
                agenda.run_pending();
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
       });

        Ok(())
    }
}
