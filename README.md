# DINGHY

## What ?

Send `cargo test` or `cargo bench` to your phone. Painlessly.

It is not a way to build portable apps, merely a way to run simple piece of
code and grab their output.

The purpose here is to make it easier for teams to unit-test and bench their
libraries on more platforms. We want to get Rust everywhere right ?

## Getting started

Depending on what is your target (iOS or Android) and your workstation, setting
up Dinghy can be more or less easy. See the setup sections for that.

```
cargo install dinghy
cargo dinghy test
cargo dinghy bench
```

## Android setup

TODO (but if you're on a mac, chances are it already works).

## iOS setup

On iOS, things are made complicated by the fact that there is no way to run
a naked executable: you need to make it an app, and sign it before sending
it to your phone. Setting up app signature requires some manipulations that
we could not find a way to dispense with through XCode. The good news is, this
is a constant time thing: *you'll have do it once in a while, but it will
cover all your Rust projects*.

Ho, and we don't need a paying account.

### Creating a signing id

You may skip most of this section if you're already setup to ship code to
your phone. 

* You'll need an Apple ID. Chances are you already have one, but you can
    an account there: https://appleid.apple.com/account .
* Go to XCode > Preferences > Accounts, and make sure your Apple ID is listed,
    or add it (`+` bottom well-hidden bottom left).
* View Details (bottom right this time)
* Click the "Create" button in front of "iOS Development" in the top half of
    this windows.
* We're done here.

### Creating a certificate

* Fire up XCode, and /create a new XCode project/.
* Pick /iOS/, /Single view application/.
* Options box:
    * Make the bundle identifier `some.unique.domainame.Dinghy`.
    * So Product Name is `Dinghy`, and `Organization identifier` is something
        that will be unique and looks like a domain name in reverse.
    * Pick your team.
    * Leave the rest alone
* Save it somewhere.
* You should get the dreadful projects settings. The only thing that is
    relevant is the "Signing" bit.
    * Plug-in your iPhone (or the device you want to run test on).
    * XCode should "repair" your project by obtaining a code-signing
        certificate for your phone.

If you're using a Apple ID free account, you will need to come back once a
week to refresh the certificate (aka "repair"). Paying account generate
longer living certificates, so you need this less often.

### Trust the certificate on your phone

On top of project window, make the target "Dinghy>Your Device", and run the
project (play button). Xcode should ask you to go to your phone settings
and Trust the certificate.

At this point, we're ready to roll.

### Debugging tips

If you got lost somewhere, here are a few hints to help you make sense of 
what is happening. This is more or less what Dinghy use when fishing for
your signing identity.

#### `security find-identity -p codesigning`

Shows you the codesigning identities available where you are. You should see 
one or more identities line, made of a long capitalize hex identifier, followed
by a "name". The name is very structured: For iOS development , its starts
with the string "iPhone Developer: ", followed by an email (for an Apple Id
account) or the name of your team. Then comes the developer identifier in
parenthesis.

#### `security find-certificate -a -c "name" -p | openssl x509 -text`

"name" is the identity name (the string between double quotes) from the command
before.

Shows you a certificate that makes the developer a part of a Team.
The certificate is signed and issued by Apple (Issuer:), but the interesting
part is the Subject line: CN= is the same developer name string, and OU= is
the actual Team identifier.

#### Look for provisioning certificates

Last but not least, we need one more certificate that proves that you and
your team has the right to deploy an App (identified by the bundle id we have
chosen while creating the project) on one (or more) devices.

These certificates are in `Library/MobileDevice/Provisioning\ Profiles`.

To read them, you'll need to do 

```
security cms -D -i  ~/Library/MobileDevice/Provisioning\ Profiles\....mobileprovision
```

The more interesting keys here are `TeamIdentifier`, `ProvisionedDevices` which
are relatively explicit, and the `Entitlements` dictionary. These entitlements
specify what the certificate is valid for, that is, signing an app identified
by a name.

Dinghy will actually scan this directory to find one that it can use (this is
where the app name being "Dinghy" plays a role).

Phew.

### command line arguments on aarch64

* there is a bug somewhere, probably in the standard library, that prevents
    command line to be received by the executable.

## Issues

