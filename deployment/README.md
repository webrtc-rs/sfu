# SFU Deployment

This plan details how to deploy the unified **SFU server** natively to a remote Fedora 42 server using Let's Encrypt SSL
certificates.

---

## 1. Domain & DNS Configuration

Configure your domain registrar's DNS Management panel to point your domain (e.g. `sfu.rs`) to your remote server:

* **A Record** pointing `@` to your server IP (e.g., `173.249.204.140`)
* **A Record** pointing `www` to your server IP (e.g., `173.249.204.140`)
* **Note**: Make sure to delete any conflicting CNAME or AAAA (IPv6) records.

---

## 2. Remote Server Prerequisites

On the remote Fedora 42 SSH terminal, install Go, Rust, Git, Rsync, and Certbot:

```bash
dnf install -y golang git rsync certbot
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

---

## 3. Obtain Let's Encrypt SSL Certificates

Obtain a free, trusted SSL certificate. Make sure port `80` is free (stop any running web servers or the sfu service
first):

```bash
systemctl stop sfu || true
certbot certonly --standalone -d sfu.rs -d www.sfu.rs --register-unsafely-without-email --agree-tos
```

Create a `/cert` directory and link the generated Let's Encrypt certificates to the expected paths:

```bash
mkdir -p /cert
ln -sf /etc/letsencrypt/live/sfu.rs/fullchain.pem /cert/cert.pem
ln -sf /etc/letsencrypt/live/sfu.rs/privkey.pem /cert/key.pem
```

---

## 4. Copy Project Files to Remote

From your **local terminal** (run this inside the project root on your MacBook), upload the code to `/opt/sfu` using
`rsync`:

```bash
rsync -avz --exclude 'target' --exclude '.git' --exclude '.idea' ./ root@173.249.204.140:/opt/sfu/
```

*Note: We use `/opt/sfu` because Fedora's default SELinux security policy prevents systemd from executing binaries
inside `/root`.*

---

## 5. Build the Server on Remote

On the **remote server**, compile the server binary and grant it execution permissions:

```bash
cd /opt/sfu
cargo build --release --example chat
chmod +x /opt/sfu/target/release/examples/chat
```

---

## 6. Run as a Systemd Background Service

To run the server continuously in the background on port `443` (HTTPS/WSS):

1. Create a service file:
   ```bash
   nano /etc/systemd/system/sfu.service
   ```

2. Add the following content:
   ```ini
   [Unit]
   Description=SFU Server
   After=network.target

   [Service]
   Type=simple
   WorkingDirectory=/opt/sfu
   ExecStart=/opt/sfu/target/release/examples/chat -s 443 --tls
   Restart=always
   RestartSec=5

   [Install]
   WantedBy=multi-user.target
   ```

3. Enable and start the service:
   ```bash
   systemctl daemon-reload
   systemctl enable --now sfu
   ```

4. Verify the status:
   ```bash
   systemctl status sfu
   ```

---

## 7. Certificate Auto-Renewal

On Fedora, installing Certbot automatically registers a background systemd timer to perform automated certificate
renewals twice a day. Since your AppRTC service runs on port `443`, port `80` is left open for Certbot to spin up its
temporary standalone validation server.

### Systemd Renewal Files

* **Timer Configuration:** `/usr/lib/systemd/system/certbot-renew.timer`
* **Renewal Service:** `/usr/lib/systemd/system/certbot-renew.service` (Runs `certbot renew --quiet --no-self-upgrade`)

---

### Step 7.1: Configure Post-Renewal Reload (Required)

The running `sfu` service must be restarted to load new certificates once they are renewed. Create a Certbot deploy
hook:

1. Create the hook script:
   ```bash
   echo -e '#!/bin/sh\nsystemctl restart sfu' > /etc/letsencrypt/renewal-hooks/deploy/restart-sfu.sh
   ```
2. Make it executable:
   ```bash
   chmod +x /etc/letsencrypt/renewal-hooks/deploy/restart-sfu.sh
   ```

---

### Step 7.2: Verify and Start the Renewal Timer

1. **Verify the timer is running**:
   Check the timer status:
   ```bash
   systemctl status certbot-renew.timer
   ```
    * **Note**: If the status says `inactive (dead)`, start and enable the timer manually:
      ```bash
      systemctl enable --now certbot-renew.timer
      ```
      Once started, the status should show `active (waiting)`.

2. **View the next scheduled run**:
   ```bash
   systemctl list-timers --all | grep certbot
   ```

3. **Simulate a renewal (Dry Run)**:
   Run this command to test the domain validation and verify the reload script runs successfully:
   ```bash
   certbot renew --dry-run
   ```