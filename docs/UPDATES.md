# Otomatik güncelleme

Otomatik güncelleme kodda hazır ama kapalı. Aşağıdaki iki ayar yapılana kadar kurulum dosyaları güncelleyici olmadan derlenir ve uygulama internete hiç bağlanmaz.

## Nasıl çalışır

1. Yayın iş akışı kurulum dosyasını **özel anahtarla** imzalar ve yayına bir `latest.json` ekler.
2. Kurulu uygulama açılışta (sonra 6 saatte bir) `releases/latest/download/latest.json` adresine bakar.
3. Daha yeni bir sürüm varsa ana ekranda "Yeni sürüm hazır" çıkar. Kullanıcı **Güncelle**'ye basmadan hiçbir şey kurulmaz.
4. İndirilen dosyanın imzası, uygulamanın içine gömülü **açık anahtarla** doğrulanır. İmza tutmazsa kurulmaz.

Denetim, Güvenlik ekranındaki **Yeni sürümleri denetle** anahtarıyla kapatılabilir.

## Açmak için gerekenler

### 1. Depo herkese açık olmalı

Uygulama `latest.json` dosyasını ve kurulum dosyasını kimlik doğrulaması olmadan indirir. Gizli depoda ikisi de erişilemez; uygulamaya erişim anahtarı gömmek güvenli değildir.

### 2. İmza anahtarı (bir kez)

Node.js kurulu bir bilgisayarda:

```
npx @tauri-apps/cli@2 signer generate -w quickswitch.key
```

Bir parola sorar ve iki dosya üretir: `quickswitch.key` (özel anahtar) ve `quickswitch.key.pub` (açık anahtar).

Depoda **Settings > Secrets and variables > Actions** altında:

| Tür | Ad | Değer |
|---|---|---|
| Variable | `UPDATE_PUBKEY` | `quickswitch.key.pub` dosyasının içeriği |
| Secret | `TAURI_SIGNING_PRIVATE_KEY` | `quickswitch.key` dosyasının içeriği |
| Secret | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Üretirken verdiğin parola |

### 3. Yeni bir sürüm yayınla

`src-tauri/tauri.conf.json` içindeki sürümü artırıp `main` dalına gönder. Yayında `latest.json` ve `.sig` dosyaları da görünüyorsa güncelleyici açılmıştır.

Bu sürümden **önce** kurulmuş kopyalar güncelleyici içermez; onlara bu sürüm bir kez elle kurulur, sonrakiler kendiliğinden gelir.

## Özel anahtar hakkında

- Özel anahtarı ele geçiren biri, kullanıcıların uygulamasının kabul edeceği sahte bir güncelleme imzalayabilir. Dosyayı ve parolayı depoya koyma, kimseyle paylaşma; GitHub Secrets dışında bir yerde yalnızca güvenli bir yedeği dursun.
- Anahtar kaybolursa kurulu kopyalar bir daha güncellenemez; yeni anahtarla derlenmiş sürümün herkese elle kurdurulması gerekir.
- Depoya yazma yetkisi olan herkes yayın çıkarabilir. Katkıcı eklerken bunu hesaba kat.
