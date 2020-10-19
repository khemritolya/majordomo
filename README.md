# Majordomo 

![](https://github.com/khemritolya/allfarbe/workflows/Rust/badge.svg)

![](https://forthebadge.com/images/badges/compatibility-ie-6.svg) ![](https://forthebadge.com/images/badges/designed-in-ms-paint.svg) ![](https://forthebadge.com/images/badges/contains-tasty-spaghetti-code.svg)

Majordomo allows you to easily integrate your project with slack, github, (eventually) email, and (potentially)  more.

Majordomo does this by allowing you to create **handlers**, which are bits of code that run when Majordomo receives certain events, including HTTP Post Requests and Slack Messages. Inside your handlers, you can use one liners to send slack messages, create github issues, and more!

### Using Majordomo

1. Think about some action you want to automate.
    <!-- TODO: update this once slack & github integration are out -->
    For example, say you wanted an endpoint to forward whatever is sent to it to slack.
    Consider the handler we would write in this situtation:

    ```rust <!-- it's not rust it's Rhai. There is no Rhai syntax highlighting :( -->
   fn handle(v) {
       slack_post("majordomo-testing-channel", v);
       v
   }
   ```

2. Obtain an API Key (for now talk to [@khemritolya](https://github.com/khemritolya) )

3. Create your handler by POST-ing to the endpoint

    ```shell script
   curl -X POST https://[addr]/upsert_handler -d "{\"uri\":\"example\", \"code\":\"fn handle(v) { slack_post("majordomo-testing-channel", v); v } \", \"api_key\":\"[your api key]\"}"
   ```

4. Make calls to the handler in your project!

    ```shell script
   curl -X POST https://[addr]/h/example -d "Hello World" 
   ```
   
   This now posts "Hello World" to "#majordomo-testing-channel" on slack. It will also respond with the json `{"status":true,"data":"Hello World"}`. If there had been any errors along the way, the status becomes false, and data contains a helpful error message! 
