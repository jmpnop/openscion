//! A blocking HTTP client for the OpenScion server API.
//!
//! Each method maps to one endpoint and returns the decoded protocol payload.
//! The client checks the protocol version on [`ServerClient::info`], so a
//! caller that starts there never talks to a server it cannot understand.
//!
//! One client holds one connection pool, so the many requests of a clone or
//! an import reuse connections instead of opening one each.

use openscion_core::{
    AncestorQuery, AncestorResult, ChangesetInfo, ChangesetRecord, CreateChangeset, DiffRequest,
    DiffResult, ErrorBody, FetchObjects, LandRequest, LandResult, LogPage, LogRequest,
    MergeBaseQuery, MergeBaseResult, ObjectId, RebaseRequest, RebaseResult, RefValue, RepoPath,
    RevertRequest, ServerInfo, StatResult, StoredObjects, TreeListing, UpdateRef, is_compatible,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Body;
use ureq::http::Response;

use crate::{Error, Result};

/// A client for one OpenScion server.
#[derive(Debug, Clone)]
pub struct ServerClient {
    base: String,
    agent: ureq::Agent,
}

impl ServerClient {
    /// Creates a client for the server at `base`, such as
    /// `http://127.0.0.1:7878`. A trailing `/` is ignored.
    ///
    /// # Examples
    ///
    /// ```
    /// # use openscion::ServerClient;
    /// let client = ServerClient::new("http://127.0.0.1:7878/");
    /// assert_eq!(client.base(), "http://127.0.0.1:7878");
    /// ```
    #[must_use]
    pub fn new(base: impl Into<String>) -> Self {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Self { base, agent }
    }

    /// The server's base URL, without a trailing `/`.
    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Fetches `GET /v1/version` and checks that the protocol binds.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Request`] when the server cannot be reached or answers
    /// with an error status, [`Error::Decode`] when the body is not a
    /// [`ServerInfo`], and [`Error::IncompatibleProtocol`] when the server's
    /// protocol version does not bind to this client's.
    pub fn info(&self) -> Result<ServerInfo> {
        let url = self.url("/v1/version");
        let info: ServerInfo = json(&url, self.agent.get(&url).call())?;
        if !is_compatible(info.protocol_version) {
            return Err(Error::IncompatibleProtocol {
                server: info.protocol_version,
            });
        }
        Ok(info)
    }

    /// Uploads a frame stream of blobs with `POST /v1/blobs`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Request`] when the server cannot be reached,
    /// [`Error::Server`] when it rejects the upload, and [`Error::Decode`]
    /// when its answer is not a [`StoredObjects`].
    pub fn upload_blobs(&self, frames: &[u8]) -> Result<StoredObjects> {
        let url = self.url("/v1/blobs");
        let request = self
            .agent
            .post(&url)
            .header("content-type", "application/octet-stream");
        json(&url, request.send(frames))
    }

    /// Fetches trees and blobs by id with `POST /v1/objects`, returning the
    /// raw frame stream in request order.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn fetch_objects(&self, ids: Vec<ObjectId>) -> Result<Vec<u8>> {
        let url = self.url("/v1/objects");
        let mut response = checked(
            &url,
            self.agent.post(&url).send_json(FetchObjects::new(ids)),
        )?;
        response
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_to_vec()
            .map_err(|error| decode_error(&url, &error))
    }

    /// Creates a changeset with `POST /v1/changesets`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn create_changeset(&self, request: &CreateChangeset) -> Result<ChangesetInfo> {
        self.post_json("/v1/changesets", request)
    }

    /// Reads a changeset with `GET /v1/changesets/{id}`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn changeset(&self, id: ObjectId) -> Result<ChangesetRecord> {
        let url = self.url(&format!("/v1/changesets/{id}"));
        json(&url, self.agent.get(&url).call())
    }

    /// Reads a ref with `GET /v1/refs/{name}`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`]; a missing ref is
    /// [`Error::Server`] with code `NotFound`.
    pub fn get_ref(&self, name: &str) -> Result<RefValue> {
        let url = self.url(&format!("/v1/refs/{name}"));
        json(&url, self.agent.get(&url).call())
    }

    /// Moves a ref by compare-and-swap with `POST /v1/refs/{name}`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`]; a lost swap is
    /// [`Error::Server`] with code `RefConflict`.
    pub fn update_ref(&self, name: &str, update: UpdateRef) -> Result<RefValue> {
        self.post_json(&format!("/v1/refs/{name}"), &update)
    }

    /// Resolves `path` in a changeset's snapshot with
    /// `GET /v1/changesets/{id}/stat`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn stat(&self, changeset: ObjectId, path: &str) -> Result<StatResult> {
        let url = self.url(&format!("/v1/changesets/{changeset}/stat"));
        json(&url, self.agent.get(&url).query("path", path).call())
    }

    /// Lists the entries under `path` (or the root) in a changeset's snapshot
    /// with `GET /v1/changesets/{id}/tree`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn list_tree(
        &self,
        changeset: ObjectId,
        path: Option<&str>,
        recursive: bool,
    ) -> Result<TreeListing> {
        let url = self.url(&format!("/v1/changesets/{changeset}/tree"));
        let mut request = self
            .agent
            .get(&url)
            .query("recursive", if recursive { "true" } else { "false" });
        if let Some(path) = path {
            request = request.query("path", path);
        }
        json(&url, request.call())
    }

    /// Reads a file's bytes from a changeset's snapshot with
    /// `GET /v1/changesets/{id}/file`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Request`] when the server cannot be reached,
    /// [`Error::Server`] with code `NotFound` when the path names nothing or a
    /// directory, and [`Error::Decode`] when the body cannot be read.
    pub fn read_file(&self, changeset: ObjectId, path: &str) -> Result<Vec<u8>> {
        let url = self.url(&format!("/v1/changesets/{changeset}/file"));
        let mut response = checked(&url, self.agent.get(&url).query("path", path).call())?;
        response
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_to_vec()
            .map_err(|error| decode_error(&url, &error))
    }

    /// Diffs one snapshot into another with `POST /v1/diff`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn diff(&self, from: ObjectId, to: ObjectId, prefixes: &[&str]) -> Result<DiffResult> {
        let prefixes = prefixes.iter().map(|p| (*p).to_owned()).collect();
        self.post_json("/v1/diff", &DiffRequest::new(from, to, prefixes))
    }

    /// Walks first-parent history with `POST /v1/log`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn log(
        &self,
        from: ObjectId,
        until: Option<ObjectId>,
        path: Option<&RepoPath>,
        limit: u64,
    ) -> Result<LogPage> {
        let mut request = LogRequest::new(from, limit);
        request.until = until;
        request.path = path.cloned();
        self.post_json("/v1/log", &request)
    }

    /// Tests whether one changeset is an ancestor of another with
    /// `POST /v1/ancestor`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn is_ancestor(&self, ancestor: ObjectId, descendant: ObjectId) -> Result<AncestorResult> {
        self.post_json("/v1/ancestor", &AncestorQuery::new(ancestor, descendant))
    }

    /// Finds a lowest common ancestor of two changesets with
    /// `POST /v1/merge-base`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`].
    pub fn merge_base(&self, a: ObjectId, b: ObjectId) -> Result<MergeBaseResult> {
        self.post_json("/v1/merge-base", &MergeBaseQuery::new(a, b))
    }

    /// Rebases a changeset onto another with `POST /v1/rebase`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`]; a conflict is [`Error::Server`]
    /// with code `MergeConflict`.
    pub fn rebase(&self, changeset: ObjectId, onto: ObjectId) -> Result<RebaseResult> {
        self.post_json("/v1/rebase", &RebaseRequest::new(changeset, onto))
    }

    /// Reverts a changeset onto another with `POST /v1/revert`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::rebase`].
    pub fn revert(
        &self,
        changeset: ObjectId,
        onto: ObjectId,
        author: &str,
        message: &str,
    ) -> Result<RebaseResult> {
        self.post_json(
            "/v1/revert",
            &RevertRequest::new(changeset, onto, author, message),
        )
    }

    /// Lands a changeset on a ref by compare-and-swap with `POST /v1/land`.
    ///
    /// # Errors
    ///
    /// As for [`ServerClient::upload_blobs`]; a conflict is [`Error::Server`]
    /// with code `MergeConflict`, and lost contention is code `RefConflict`.
    pub fn land(
        &self,
        changeset: ObjectId,
        ref_name: &str,
        max_attempts: u32,
    ) -> Result<LandResult> {
        self.post_json(
            "/v1/land",
            &LandRequest::new(changeset, ref_name, max_attempts),
        )
    }

    fn post_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let url = self.url(path);
        json(&url, self.agent.post(&url).send_json(body))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// Turns a transport failure or an error status into an [`Error`].
fn checked(
    url: &str,
    response: std::result::Result<Response<Body>, ureq::Error>,
) -> Result<Response<Body>> {
    let mut response = response.map_err(|error| Error::Request {
        url: url.to_owned(),
        message: error.to_string(),
    })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    match response.body_mut().read_json::<ErrorBody>() {
        Ok(body) => Err(Error::Server {
            url: url.to_owned(),
            status: status.as_u16(),
            code: body.code,
            message: body.message,
        }),
        Err(_) => Err(Error::Request {
            url: url.to_owned(),
            message: format!("server answered {status}"),
        }),
    }
}

fn json<T: DeserializeOwned>(
    url: &str,
    response: std::result::Result<Response<Body>, ureq::Error>,
) -> Result<T> {
    checked(url, response)?
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()
        .map_err(|error| decode_error(url, &error))
}

fn decode_error(url: &str, error: &ureq::Error) -> Error {
    Error::Decode {
        url: url.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod test;
