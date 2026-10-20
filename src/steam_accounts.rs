use super::*;
use crate::providers::steam::{Client, Counter, observe};

impl FreshConnectionsService {
    pub(super) async fn start_steam(
        &self,
        user_id: Uuid,
    ) -> Result<StartConnectionResponse, FreshConnectionError> {
        if !self.steam.enabled() {
            return Err(FreshConnectionError::NotConfigured(
                "Steam Web API key missing".into(),
            ));
        }
        let state = crate::crypto::random_token()?;
        let mut redirect = url::Url::parse(
            &self
                .google_redirect()?
                .replace("/google/callback", "/steam/callback"),
        )
        .map_err(|_| FreshConnectionError::Invalid("Invalid callback URL".into()))?;
        redirect.query_pairs_mut().append_pair("state", &state);
        let redirect = redirect.to_string();
        let authorization_url = Client::auth_url(&redirect)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:steam"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE vox_connection_setups SET status='cancelled',verifier_ciphertext=NULL WHERE user_id=$1 AND connector_id='steam' AND status IN ('pending','exchanging')").bind(user_id).execute(&mut *tx).await?;
        let id = sqlx::query_scalar("INSERT INTO vox_connection_setups(user_id,connector_id,state_token,redirect_uri,consented_at) VALUES($1,'steam',$2,$3,now()) RETURNING id").bind(user_id).bind(state).bind(redirect).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(StartConnectionResponse {
            setup_id: Some(id),
            connection_id: None,
            authorization_url: Some(authorization_url),
            status: "pending".into(),
        })
    }
    pub async fn handle_steam_callback(
        &self,
        params: &BTreeMap<String, String>,
    ) -> Result<Uuid, FreshConnectionError> {
        let state = params
            .get("state")
            .ok_or(FreshConnectionError::Unauthorized)?;
        // A one-use, expiring state binds the assertion to the consenting Vox user.
        let setup=sqlx::query("UPDATE vox_connection_setups SET status='exchanging' WHERE connector_id='steam' AND state_token=$1 AND status='pending' AND expires_at>now() AND consented_at IS NOT NULL RETURNING *").bind(state).fetch_optional(&self.pool).await?.ok_or(FreshConnectionError::Unauthorized)?;
        let result = self.complete_steam(params, &setup).await;
        if result.is_err() {
            sqlx::query("UPDATE vox_connection_setups SET status='failed',error='authorization_failed' WHERE id=$1 AND status='exchanging'").bind(setup.get::<Uuid,_>("id")).execute(&self.pool).await?;
        }
        result
    }
    async fn complete_steam(
        &self,
        params: &BTreeMap<String, String>,
        setup: &sqlx::postgres::PgRow,
    ) -> Result<Uuid, FreshConnectionError> {
        let user: Uuid = setup.get("user_id");
        let setup_id: Uuid = setup.get("id");
        let account = self
            .steam
            .verify(params, &setup.get::<String, _>("redirect_uri"))
            .await?;
        let games = self.steam.games(&account).await?;
        let (counters, _) = observe(&account, &BTreeMap::new(), &games, Utc::now());
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user}:steam"))
            .execute(&mut *tx)
            .await?;
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM vox_connection_setups WHERE id=$1 AND status='exchanging' AND expires_at>now() FOR UPDATE").bind(setup_id).fetch_optional(&mut *tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        let id:Uuid=sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,account_id,account_display_id,authorization_state,sync_timeline,assistant_read,metadata,consented_at,last_synced_at,next_sync_at) VALUES($1,'steam',$2,$2,'authorized',true,true,$3,now(),now(),now()+interval '15 minutes') ON CONFLICT(user_id,connector_id) DO UPDATE SET generation=gen_random_uuid(),credential_generation=gen_random_uuid(),lease_token=NULL,lease_until=NULL,account_id=EXCLUDED.account_id,account_display_id=EXCLUDED.account_display_id,access_ciphertext=NULL,refresh_ciphertext=NULL,access_expires_at=NULL,authorization_state='authorized',sync_timeline=true,assistant_read=true,metadata=EXCLUDED.metadata,consented_at=now(),last_synced_at=now(),next_sync_at=now()+interval '15 minutes',failure_code=NULL,failure_count=0,updated_at=now() RETURNING id")
            .bind(user).bind(&account).bind(json!({"auth_type":"openid","steam_counters":counters})).fetch_one(&mut *tx).await?;
        sqlx::query(
            "UPDATE vox_connection_setups SET status='authorized',connection_id=$2 WHERE id=$1",
        )
        .bind(setup_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }
    pub(super) async fn sync_steam(
        &self,
        user: Uuid,
        id: Uuid,
    ) -> Result<usize, FreshConnectionError> {
        let conn = self.claim(user, id).await?;
        let result = self.sync_steam_claimed(user, id, &conn).await;
        if let Err(ref e) = result {
            self.release_failure(id, &conn, e).await?;
        }
        result
    }
    async fn sync_steam_claimed(
        &self,
        user: Uuid,
        id: Uuid,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<usize, FreshConnectionError> {
        if conn.get::<String, _>("connector_id") != "steam" {
            return Err(FreshConnectionError::Unauthorized);
        }
        let account: String = conn.get("account_id");
        let games = self.steam.games(&account).await?;
        let mut metadata: Value = conn.get("metadata");
        let previous: BTreeMap<u32, Counter> = if metadata["baseline_required"] == true {
            BTreeMap::new()
        } else {
            serde_json::from_value(metadata["steam_counters"].clone())
                .map_err(|_| FreshConnectionError::Invalid("Invalid Steam checkpoint".into()))?
        };
        let (counters, activities) = observe(&account, &previous, &games, Utc::now());
        let mut tx = self.pool.begin().await?;
        self.validate_commit(&mut tx, user, id, conn).await?;
        let count = self
            .ingestor
            .personal_activity(&mut tx, user, id, "steam", &activities)
            .await?;
        metadata["steam_counters"] = json!(counters);
        metadata["baseline_required"] = json!(false);
        sqlx::query("UPDATE vox_connections SET metadata=$2,last_synced_at=now(),next_sync_at=now()+interval '15 minutes',lease_token=NULL,lease_until=NULL,failure_count=0,failure_code=NULL,updated_at=now() WHERE id=$1").bind(id).bind(metadata).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(count)
    }
    pub(super) async fn read_steam(
        &self,
        user: Uuid,
        id: Uuid,
        limit: usize,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<Value, FreshConnectionError> {
        let account: String = conn.get("account_id");
        let mut games = self.steam.games(&account).await?;
        games.sort_by_key(|g| std::cmp::Reverse(g.recent_minutes.unwrap_or(0)));
        let total = games.len();
        games.truncate(limit);
        let mut tx = self.pool.begin().await?;
        self.validate_personal_read(&mut tx, user, id, conn).await?;
        sqlx::query("UPDATE vox_connections SET lease_token=NULL,lease_until=NULL WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"connector_id":"steam","observed_at":Utc::now(),"complete":total<=limit,"total_games":total,"limit":limit,"items":games,"session_times_known":false,"timing":"cumulative_playtime_minutes"}),
        )
    }
}
