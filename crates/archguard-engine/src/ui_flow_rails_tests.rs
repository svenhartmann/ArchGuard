use super::*;
use archguard_core::{AccessState as A, Condition as C, Config, DiagnosticSeverity};

fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let config: Config = toml::from_str(&format!("[views.access]\n{access}")).unwrap();
    extract_with(&ExtractContext::new(dir.path(), &config))
}

fn state(flow: &UiFlow, method: &str, path: &str) -> A {
    let found: Vec<_> = (flow.endpoints.iter())
        .filter(|e| e.method == method && e.path == path)
        .collect();
    assert_eq!(found.len(), 1, "{method} {path} in {:?}", flow.endpoints);
    found[0].access.clone()
}

fn has(flow: &UiFlow, severity: DiagnosticSeverity, part: &str) -> bool {
    (flow.diagnostics.iter()).any(|d| d.severity == severity && d.message.contains(part))
}

fn auth() -> A {
    A::Protected(C::Authenticated)
}

fn pol(n: &str) -> A {
    A::Protected(C::Policy(n.into()))
}

fn unknown(n: &str) -> A {
    A::Protected(C::Unknown(n.into()))
}

const APP: &str = "class ApplicationController < ActionController::Base\n  before_action :authenticate_user!\nend\n";

fn routes(body: &str) -> String {
    format!("Rails.application.routes.draw do\n  devise_for :users\n{body}end\n")
}

#[test]
fn f01_devise_before_action() {
    let r = routes("  resources :posts, only: [:index]\n");
    let flow = run(
        &[
            ("config/routes.rb", &r),
            ("app/controllers/application_controller.rb", APP),
            (
                "app/controllers/posts_controller.rb",
                "class PostsController < ApplicationController\n  def index\n  end\nend\n",
            ),
        ],
        "",
    );
    assert_eq!(state(&flow, "GET", "/posts"), auth());
    assert_eq!(
        flow.endpoints[0].handler.as_deref(),
        Some("PostsController#index")
    );
}

const PAGES_ROUTES: &str =
    "  get \"home\", to: \"pages#home\"\n  get \"about\", to: \"pages#about\"\n";

#[test]
fn f02_skip_before_action_opens_only_listed() {
    let r = routes(PAGES_ROUTES);
    let flow = run(
        &[
            ("config/routes.rb", &r),
            ("app/controllers/application_controller.rb", APP),
            (
                "app/controllers/pages_controller.rb",
                "class PagesController < ApplicationController\n  skip_before_action :authenticate_user!, only: [:home]\nend\n",
            ),
        ],
        "",
    );
    assert_eq!(state(&flow, "GET", "/home"), A::Open);
    assert_eq!(state(&flow, "GET", "/about"), auth());
}

#[test]
fn f03_reregistration_replaces_parent_callback() {
    let r = routes("  resources :posts, only: [:index, :edit]\n");
    let flow = run(
        &[
            ("config/routes.rb", &r),
            ("app/controllers/application_controller.rb", APP),
            (
                "app/controllers/posts_controller.rb",
                "class PostsController < ApplicationController\n  before_action :authenticate_user!, only: [:edit]\nend\n",
            ),
        ],
        "",
    );
    assert_eq!(state(&flow, "GET", "/posts/:id/edit"), auth());
    assert_eq!(state(&flow, "GET", "/posts"), A::Undeclared);
    assert!(has(&flow, DiagnosticSeverity::Note, "replaces"));
}

#[test]
fn f04_project_defined_guard_has_no_meaning_unless_listed() {
    let files = [
        (
            "config/routes.rb",
            "Rails.application.routes.draw do\n  get \"x\", to: \"pages#x\"\nend\n",
        ),
        (
            "app/controllers/application_controller.rb",
            "class ApplicationController\n  before_action :authenticate_user!\n  def authenticate_user!\n    redirect_to \"/login\" unless session[:u]\n  end\nend\n",
        ),
        (
            "app/controllers/pages_controller.rb",
            "class PagesController < ApplicationController\nend\n",
        ),
    ];
    let flow = run(&files, "");
    assert_eq!(state(&flow, "GET", "/x"), A::Undeclared);
    assert!(has(&flow, DiagnosticSeverity::Note, "authenticate_user!"));
    let flow = run(&files, "\"authenticate_user!\" = \"authenticated\"\n");
    assert_eq!(state(&flow, "GET", "/x"), auth());
}

#[test]
fn f05_conditional_guard_is_unknown() {
    let flow = run(
        &[
            (
                "config/routes.rb",
                "Rails.application.routes.draw do\n  get \"x\", to: \"pages#x\"\nend\n",
            ),
            (
                "app/controllers/pages_controller.rb",
                "class PagesController < ApplicationController\n  before_action :require_admin, if: -> { params[:scope] == \"all\" }\nend\n",
            ),
        ],
        "require_admin = \"role:admin\"\n",
    );
    assert!(
        matches!(state(&flow, "GET", "/x"), A::Protected(C::Unknown(t)) if t.contains("require_admin"))
    );
    assert!(has(&flow, DiagnosticSeverity::Gap, "require_admin"));
    assert!(!flow.complete);
}

#[test]
fn f06_devise_authenticate_block() {
    let r = routes(
        "  authenticate :user do\n    resources :reports, only: [:index]\n  end\n  authenticate :user, ->(u) { u.admin? } do\n    mount Sidekiq::Web => \"/sidekiq\"\n  end\n",
    );
    let flow = run(&[("config/routes.rb", &r)], "");
    assert_eq!(state(&flow, "GET", "/reports"), auth());
    match state(&flow, "ANY", "/sidekiq") {
        A::Protected(C::All(items)) => {
            assert_eq!(items[0], C::Authenticated);
            assert!(matches!(&items[1], C::Unknown(t) if t.contains("u.admin?")));
        }
        other => panic!("{other:?}"),
    }
    assert!(has(&flow, DiagnosticSeverity::Gap, "u.admin?"));
}

#[test]
fn f07_authenticated_and_unauthenticated_roots() {
    let r = routes(
        "  authenticated :user do\n    root to: \"dashboard#show\", as: :authenticated_root\n  end\n  root to: \"landing#show\"\n",
    );
    let flow = run(&[("config/routes.rb", &r)], "");
    let roots: Vec<_> = (flow.endpoints.iter())
        .filter(|e| e.path == "/")
        .map(|e| (e.handler.clone().unwrap(), e.access.clone()))
        .collect();
    assert!(roots.contains(&("DashboardController#show".into(), auth())));
    assert!(roots.contains(&("LandingController#show".into(), A::Undeclared)));
}

#[test]
fn f08_constraints() {
    let r = routes(
        "  constraints(Constraints::Admin) do\n    resources :flags, only: [:index]\n  end\n  constraints(subdomain: \"api\") do\n    resources :things, only: [:index]\n  end\n",
    );
    let flow = run(&[("config/routes.rb", &r)], "");
    assert_eq!(state(&flow, "GET", "/flags"), unknown("Constraints::Admin"));
    assert_eq!(state(&flow, "GET", "/things"), A::Undeclared);
    assert!(has(&flow, DiagnosticSeverity::Gap, "Constraints::Admin"));
    let flow = run(
        &[("config/routes.rb", &r)],
        "\"Constraints::Admin\" = \"role:admin\"\n",
    );
    assert_eq!(
        state(&flow, "GET", "/flags"),
        A::Protected(C::Role("admin".into()))
    );
}

const AUTH_CONCERN: &str = "module Authentication\n  extend ActiveSupport::Concern\n  included do\n    before_action :require_authentication\n  end\n  class_methods do\n    def allow_unauthenticated_access(**options)\n      skip_before_action :require_authentication, **options\n    end\n  end\n  private\n    def require_authentication\n      resume_session || request_authentication\n    end\nend\n";

#[test]
fn f09_rails8_authentication_concern() {
    let flow = run(
        &[
            (
                "config/routes.rb",
                "Rails.application.routes.draw do\n  resource :session, only: [:new, :create, :destroy]\nend\n",
            ),
            ("app/controllers/concerns/authentication.rb", AUTH_CONCERN),
            (
                "app/controllers/application_controller.rb",
                "class ApplicationController < ActionController::Base\n  include Authentication\nend\n",
            ),
            (
                "app/controllers/sessions_controller.rb",
                "class SessionsController < ApplicationController\n  allow_unauthenticated_access only: %i[ new create ]\nend\n",
            ),
        ],
        "",
    );
    assert_eq!(state(&flow, "GET", "/session/new"), A::Open);
    assert_eq!(state(&flow, "POST", "/session"), A::Open);
    assert_eq!(state(&flow, "DELETE", "/session"), auth());
}

const PUNDIT_APP: &str =
    "class ApplicationController < ActionController::Base\n  include Pundit::Authorization\nend\n";

fn pundit(controller: &str, extra_routes: &str) -> UiFlow {
    let r = format!("Rails.application.routes.draw do\n{extra_routes}end\n");
    run(
        &[
            ("config/routes.rb", &r),
            ("Gemfile.lock", "    pundit (2.3.0)\n"),
            ("app/controllers/application_controller.rb", PUNDIT_APP),
            ("app/controllers/posts_controller.rb", controller),
        ],
        "",
    )
}

#[test]
fn f10_pundit_authorize() {
    let flow = pundit(
        "class PostsController < ApplicationController\n  def update\n    @post = Post.find(params[:id])\n    authorize @post\n  end\n  def publish\n    @post = Post.find(params[:id])\n    authorize @post, :publish?\n  end\n  def dash\n    authorize :dashboard, :show?\n  end\n  def adm\n    @post = Post.find(params[:id])\n    authorize [:admin, @post]\n  end\n  def mystery\n    authorize something\n  end\nend\n",
        "  patch \"p\", to: \"posts#update\"\n  get \"pub\", to: \"posts#publish\"\n  get \"dash\", to: \"posts#dash\"\n  get \"adm\", to: \"posts#adm\"\n  get \"mys\", to: \"posts#mystery\"\n",
    );
    assert_eq!(state(&flow, "PATCH", "/p"), pol("PostPolicy#update?"));
    assert_eq!(state(&flow, "GET", "/pub"), pol("PostPolicy#publish?"));
    assert_eq!(state(&flow, "GET", "/dash"), pol("DashboardPolicy#show?"));
    assert_eq!(state(&flow, "GET", "/adm"), pol("Admin::PostPolicy#adm?"));
    assert!(matches!(
        state(&flow, "GET", "/mys"),
        A::Protected(C::Unknown(_))
    ));
    assert!(!flow.complete);
}

#[test]
fn f11_policy_scope_and_verify_authorized_protect_nothing() {
    let flow = pundit(
        "class PostsController < ApplicationController\n  after_action :verify_authorized, except: :index\n  def index\n    @posts = policy_scope(Post)\n  end\nend\n",
        "  get \"posts\", to: \"posts#index\"\n",
    );
    assert_eq!(state(&flow, "GET", "/posts"), A::Undeclared);
}

#[test]
fn f12_skip_authorization_is_open_only_without_callbacks() {
    let body = "def show\n    skip_authorization\n  end\n";
    let bare = pundit(
        &format!("class PostsController < ApplicationController\n  {body}end\n"),
        "  get \"s\", to: \"posts#show\"\n",
    );
    assert_eq!(state(&bare, "GET", "/s"), A::Open);
    let guarded = pundit(
        &format!(
            "class PostsController < ApplicationController\n  before_action :authenticate_user!\n  {body}end\n"
        ),
        "  devise_for :users\n  get \"s\", to: \"posts#show\"\n",
    );
    assert_eq!(state(&guarded, "GET", "/s"), auth());
}

#[test]
fn f13_conditional_authorize_is_no_condition() {
    let flow = pundit(
        "class PostsController < ApplicationController\n  def show\n    @post = Post.find(1)\n    authorize @post if params[:strict]\n  end\nend\n",
        "  get \"s\", to: \"posts#show\"\n",
    );
    assert_eq!(state(&flow, "GET", "/s"), A::Undeclared);
    assert!(has(&flow, DiagnosticSeverity::Note, "authorize"));
}

fn cancan(controller: &str) -> UiFlow {
    run(
        &[
            (
                "config/routes.rb",
                "Rails.application.routes.draw do\n  resources :articles, only: [:index, :show]\nend\n",
            ),
            ("Gemfile.lock", "    cancancan (3.5.0)\n"),
            ("app/controllers/articles_controller.rb", controller),
        ],
        "",
    )
}

#[test]
fn f14_cancan_resource_macros() {
    let flow = cancan(
        "class ArticlesController < ApplicationController\n  load_and_authorize_resource\nend\n",
    );
    assert_eq!(state(&flow, "GET", "/articles"), pol("index Article"));
    assert_eq!(state(&flow, "GET", "/articles/:id"), pol("show Article"));
    let flow = cancan(
        "class ArticlesController < ApplicationController\n  load_and_authorize_resource except: :index\nend\n",
    );
    assert_eq!(state(&flow, "GET", "/articles"), A::Undeclared);
    assert_eq!(state(&flow, "GET", "/articles/:id"), pol("show Article"));
    let flow = cancan("class ArticlesController < ApplicationController\n  load_resource\nend\n");
    assert_eq!(state(&flow, "GET", "/articles"), A::Undeclared);
}

#[test]
fn f15_cancan_authorize_bang() {
    let flow = cancan(
        "class ArticlesController < ApplicationController\n  def show\n    @article = Article.find(params[:id])\n    authorize! :edit, @article\n  end\n  def index\n    authorize! :edit, @thing\n  end\nend\n",
    );
    assert_eq!(state(&flow, "GET", "/articles/:id"), pol("edit Article"));
    assert!(matches!(
        state(&flow, "GET", "/articles"),
        A::Protected(C::Unknown(_))
    ));
    assert!(!flow.complete);
}

#[test]
fn f18_authorize_bang_needs_one_library() {
    let with = |lock| -> UiFlow {
        run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  get \"s\", to: \"posts#show\"\nend\n",
                ),
                ("Gemfile.lock", lock),
                (
                    "app/controllers/posts_controller.rb",
                    "class PostsController < ApplicationController\n  def show\n    @post = Post.find(1)\n    authorize! @post\n  end\nend\n",
                ),
            ],
            "",
        )
    };
    assert_eq!(
        state(&with("    action_policy (0.7.0)\n"), "GET", "/s"),
        pol("PostPolicy#show?")
    );
    let both = with("    action_policy (0.7.0)\n    cancancan (3.5.0)\n");
    assert!(matches!(
        state(&both, "GET", "/s"),
        A::Protected(C::Unknown(_))
    ));
    assert!(!both.complete);
}

#[test]
fn f19_http_basic_authenticate_with() {
    let with = |app: &str| {
        run(
            &[
                (
                    "config/routes.rb",
                    "Rails.application.routes.draw do\n  resources :events, only: [:index, :create]\nend\n",
                ),
                ("app/controllers/application_controller.rb", app),
                (
                    "app/controllers/events_controller.rb",
                    "class EventsController < ApplicationController\n  http_basic_authenticate_with name: ENV[\"U\"], password: ENV[\"P\"], except: :index\nend\n",
                ),
            ],
            "",
        )
    };
    let flow = with("class ApplicationController\nend\n");
    assert_eq!(state(&flow, "GET", "/events"), A::Undeclared);
    assert_eq!(state(&flow, "POST", "/events"), auth());
    let flow = with(
        "class ApplicationController\n  def self.http_basic_authenticate_with(**options)\n  end\nend\n",
    );
    assert_eq!(state(&flow, "GET", "/events"), A::Undeclared);
    assert!(matches!(
        state(&flow, "POST", "/events"),
        A::Protected(C::Unknown(_))
    ));
    assert!(!flow.complete);
}

#[test]
fn f20_guard_inherited_through_class_and_concern() {
    let flow = run(
        &[
            (
                "config/routes.rb",
                "Rails.application.routes.draw do\n  devise_for :users\n  namespace :admin do\n    resources :users, only: [:index]\n  end\nend\n",
            ),
            (
                "app/controllers/concerns/guarded.rb",
                "module Guarded\n  extend ActiveSupport::Concern\n  included do\n    before_action :authenticate_user!\n  end\nend\n",
            ),
            (
                "app/controllers/admin/base_controller.rb",
                "module Admin\n  class BaseController < ApplicationController\n    include Guarded\n  end\nend\n",
            ),
            (
                "app/controllers/admin/users_controller.rb",
                "class Admin::UsersController < Admin::BaseController\nend\n",
            ),
        ],
        "",
    );
    assert_eq!(state(&flow, "GET", "/admin/users"), auth());
    assert_eq!(
        flow.endpoints[0].handler.as_deref(),
        Some("Admin::UsersController#index")
    );
}

#[test]
fn skip_never_opens_next_to_an_unreadable_guard() {
    let r = routes(PAGES_ROUTES);
    let flow = run(
        &[
            ("config/routes.rb", &r),
            (
                "app/controllers/application_controller.rb",
                "class ApplicationController\n  before_action :authenticate_user!\n  before_action :require_admin\n  def require_admin\n    redirect_to root_path unless current_user.admin?\n  end\nend\n",
            ),
            (
                "app/controllers/pages_controller.rb",
                "class PagesController < ApplicationController\n  skip_before_action :authenticate_user!, only: [:home]\nend\n",
            ),
        ],
        "",
    );
    assert_ne!(state(&flow, "GET", "/home"), A::Open);
    assert!(has(&flow, DiagnosticSeverity::Gap, "require_admin"));
}

#[test]
fn nested_resources_member_collection_and_scopes() {
    let r = "Rails.application.routes.draw do\n  scope \"/v1\" do\n    resources :posts, only: [:show] do\n      resources :comments, only: [:index]\n      member do\n        post :publish\n      end\n      collection do\n        get :search\n      end\n    end\n  end\nend\n";
    let flow = run(&[("config/routes.rb", r)], "");
    for (m, p) in [
        ("GET", "/v1/posts/:id"),
        ("GET", "/v1/posts/:post_id/comments"),
        ("POST", "/v1/posts/:id/publish"),
        ("GET", "/v1/posts/search"),
    ] {
        state(&flow, m, p);
    }
}
