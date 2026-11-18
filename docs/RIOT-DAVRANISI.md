# Riot Client hakkında varsayımlar

QuickSwitch, Riot'un belgelemediği davranışlara dayanır. Testlerdeki sahte Riot Client bu varsayımları taklit eder; Riot bir şeyi değiştirirse testler geçmeye devam eder ama uygulama bozulur. Bu sayfa her varsayımı, dayanağını ve kodda nerede olduğunu tutar. Bir sorun çıktığında ilk bakılacak yer burasıdır.

Son gözden geçirme: Kasım 2026. Kaynaklar aynı yöntemi kullanan açık kaynak projelerin belgeleridir; hiçbiri Riot'un resmî belgesi değildir.

| Varsayım | Dayanak | Kodda |
|---|---|---|
| "Oturumu açık tut" oturumu `Riot Client/Data/RiotGamesPrivateSettings.yaml` içinde durur ve istemci düzgün kapanırken yazılır | accshift, Alt Hop, V-Shift (5.0.2 notu: çerezler artık `Data/Cookies` yerine bu dosyada) | `store.rs` `session_paths`, `switcher.rs` `close_client` |
| Kutu işaretlenmeden açılan oturum geçicidir, dosyaya yazılmaz | accshift wiki | `capture_new` (`NotPersisted`) |
| Riot Client'ta **Çıkış yap**, oturumu Riot tarafında geçersiz kılar; kayıtlı kopya da ölür | league-account-switcher, game-account-switcher, riot-account-switcher | `new_login` çıkış yapmaz, yalnızca dosyaları kaldırır; README uyarısı |
| Kullanılmayan oturum bir süre sonra sonlanır; süre açıklanmıyor (bir proje 1–3 hafta diyor). Şifre değişimi ve "tüm cihazlardan çık" da sonlandırır | league-account-switcher, Alt Hop | `stale_after_days` (varsayılan 7), `await_login` |
| Yerel arayüzle şifreyle giriş (`PUT /rso-auth/v1/session/credentials`) artık tek başına yetmez: giriş hCaptcha ister | eski örnekler bu yolu kullanıyor; güncel `riot-login` projesi hCaptcha'yı tarayıcıda çözdürüyor | Bu yüzden şifre pencereye yazılır (`typer.rs`) |
| Oturum dosyası yeni biçimde hesap kimliğini açıkça taşımaz | Depodaki gözlem (`identity.rs` başlığı); doğrulayan dış kaynak bulunmadı | `identity.rs` `token_account` (ipucu), `switcher.rs` `observe_live` |
| Giriş durumu `/rso-auth/v1/session`, kimlik `/riot-client-auth/v1/userinfo` (`sub` = hesap kimliği, UUID biçiminde) | Topluluk notları (VALORANT kimlik akışı) | `api.rs` |

| Şifreli girişin neden geçmediği `/rso-authenticator/v1/authentication` ucundan okunur: `type: multifactor` doğrulama kodu, dolu `error` alanı ret (içinde "captcha" ya da "rate" geçiyorsa o) | League Accounts'un kaynak kodu (v3.5.0); biz gerçek istemcide görmedik | `api.rs` `auth_problem_from`, `switcher.rs` `await_login` |
| League banı kimlik bilgisindeki `ban.restrictions` listesinde durur: `scope: lol`, `type: TIME_BAN` (`dat.expirationMillis`) ya da `PERMANENT_BAN` | League Accounts'un kaynak kodu; biz görmedik | `api.rs` `league_ban` |
| Riot Client yönetici olarak çalışmaya ayarlıysa başlatma Windows hatası 740 ile reddedilir | Windows davranışı | `switcher.rs` `launch` |

## Bilinen açık sorular

- Yeni biçimli dosyadaki tokenlardan biri hesabı adlandıran bir JWT mi? Öyleyse `token_account` kapalı istemcinin oturumunu her zaman doğru hesaba yazar. Gerçek bir dosyada denenmedi; ipucu yalnızca bilinen bir hesabı tanımak için kullanılır, hiçbir kaydı engellemez.
- Oturumun tam ömrü. `quickswitch.log` içindeki "giriş kabul edildi / gerekli" satırları ile hesabın kaç gündür açılmadığı karşılaştırılarak ölçülebilir.
- Klavyesiz giriş (0.4.4, `typer.rs` içindeki `uia`): giriş alanları Windows UI Automation ile doldurulur, "Oturumu açık tut" işaretlenir, giriş düğmesine basılır. Üç varsayıma dayanır ve hiçbiri gerçek istemcide doğrulanmadı: (1) sayfa erişilebilirlik ağacını istenince kurar ve iki düzenleme alanı gösterir, biri şifre alanıdır; (2) alanlara bu yolla yazılan değeri sayfa kabul eder; (3) giriş düğmesi alanlar boşken basılamaz, dolunca basılabilir (düğme adıyla değil bu davranışıyla bulunur). Biri tutmazsa alanlar boşaltılıp klavye yoluna dönülür ve `quickswitch.log` hangisinin tutmadığını yazar.
